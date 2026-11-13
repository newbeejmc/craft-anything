//! Cancellation closes the actual connection, including a quiet HTTP or TLS read.
use crate::model::Stop;
use craft_anything_evidence::ToolError;
use craft_anything_platform::sync::Mutex;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, Weak, mpsc};
use std::time::{Duration, Instant};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, RustlsConnector, Transport,
};

const POLL: Duration = Duration::from_millis(25);
const MAX_CONNECTION_WORKERS: usize = 32;
static CONNECTION_WORKERS: AtomicUsize = AtomicUsize::new(0);
struct ConnectionSlot;
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        CONNECTION_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct Connections {
    stop: Stop,
    sockets: Mutex<Vec<Weak<TcpStream>>>,
}

fn interrupted() -> ureq::Error {
    // Read/write helpers and TLS retry Interrupted automatically. Cancellation is final.
    io::Error::new(io::ErrorKind::ConnectionAborted, "The model request was stopped.").into()
}

impl Connections {
    fn register(&self, socket: &Arc<TcpStream>) -> Result<(), ureq::Error> {
        let mut sockets = self.sockets.lock().unwrap_or_else(PoisonError::into_inner);
        if self.stop.is_stopped() {
            let _ = socket.shutdown(Shutdown::Both);
            return Err(interrupted());
        }
        sockets.retain(|socket| socket.strong_count() > 0);
        sockets.push(Arc::downgrade(socket));
        Ok(())
    }
}

/// No HTTP worker is abandoned: closing its socket wakes reads, writes and TLS handshakes.
pub(super) fn agent(config: ureq::config::Config, stop: &Stop) -> Result<ureq::Agent, ToolError> {
    stop.check()?;
    let connections = Arc::new(Connections { stop: stop.clone(), sockets: Mutex::default() });
    let weak = Arc::downgrade(&connections);
    std::thread::Builder::new()
        .name("ai-http-cancel".into())
        .spawn(move || {
            loop {
                let Some(connections) = weak.upgrade() else { break };
                if connections.stop.is_stopped() {
                    let sockets = connections.sockets.lock().unwrap_or_else(PoisonError::into_inner);
                    for socket in sockets.iter().filter_map(Weak::upgrade) {
                        let _ = socket.shutdown(Shutdown::Both);
                    }
                    break;
                }
                drop(connections);
                std::thread::park_timeout(POLL);
            }
        })
        .map_err(|error| ToolError::internal(format!("Couldn't start model-request cancellation: {error}")))?;
    // Keep ureq's CONNECT proxy and certificate-verified TLS implementations.
    let connector = ().chain(ConnectProxyConnector::default()).chain(SocketConnector(connections)).chain(RustlsConnector::default());
    Ok(ureq::Agent::with_parts(config, connector, StopResolver(stop.clone())))
}

/// DNS and TCP connect cannot be interrupted portably in safe std. Only those bounded
/// operations run separately; a cancelled result is dropped without sending an HTTP request.
fn wait<T: Send + 'static>(stop: &Stop, timeout: NextTimeout, work: impl FnOnce() -> Result<T, ureq::Error> + Send + 'static) -> Result<T, ureq::Error> {
    if stop.is_stopped() {
        return Err(interrupted());
    }
    CONNECTION_WORKERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| (count < MAX_CONNECTION_WORKERS).then(|| count + 1))
        .map_err(|_| io::Error::other("Earlier name lookups or connections are still finishing. Try again shortly."))?;
    let slot = ConnectionSlot;
    let worker_stop = stop.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("ai-http-connect".into()).spawn(move || {
        let _slot = slot;
        let result = if worker_stop.is_stopped() { Err(interrupted()) } else { work() };
        let _ = tx.send(result);
    })?;
    let limit = timeout.not_zero().map(|d| *d).unwrap_or(Duration::from_secs(15));
    let started = Instant::now();
    loop {
        if stop.is_stopped() {
            return Err(interrupted());
        }
        let remaining = limit.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(ureq::Error::Timeout(timeout.reason));
        }
        match rx.recv_timeout(remaining.min(POLL)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("The connection worker ended unexpectedly.").into()),
        }
    }
}

#[derive(Debug)]
struct StopResolver(Stop);
impl Resolver for StopResolver {
    fn resolve(&self, uri: &ureq::http::Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let (uri, config) = (uri.clone(), config.clone());
        wait(&self.0, timeout, move || {
            // The bounded worker owns this OS lookup. Avoid another detached resolver thread.
            let timeout = NextTimeout { after: ureq::unversioned::transport::time::Duration::NotHappening, ..timeout };
            DefaultResolver::default().resolve(&uri, &config, timeout)
        })
    }
}

#[derive(Debug)]
struct SocketConnector(Arc<Connections>);
impl<In: Transport> Connector<In> for SocketConnector {
    type Out = Either<In, SocketTransport>;
    fn connect(&self, details: &ConnectionDetails<'_>, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        if let Some(chained) = chained {
            return Ok(Some(Either::A(chained)));
        }
        let addresses = details.addrs.clone();
        let stop = self.0.stop.clone();
        let timeout = details.timeout;
        let limit = timeout.not_zero().map(|d| *d).unwrap_or(Duration::from_secs(15));
        let no_delay = details.config.no_delay();
        let socket = wait(&self.0.stop, timeout, move || {
            let started = Instant::now();
            let mut error = ureq::Error::HostNotFound;
            let count = u32::try_from(addresses.len()).unwrap_or(u32::MAX).max(1);
            for address in &addresses {
                if stop.is_stopped() {
                    return Err(interrupted());
                }
                let remaining = limit.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(ureq::Error::Timeout(timeout.reason));
                }
                match TcpStream::connect_timeout(address, remaining.min((limit / count).max(Duration::from_millis(1)))) {
                    Ok(socket) => {
                        socket.set_nodelay(no_delay)?;
                        return Ok(socket);
                    }
                    Err(e) => error = e.into(),
                }
            }
            Err(error)
        })?;
        let socket = Arc::new(socket);
        self.0.register(&socket)?;
        let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
        Ok(Some(Either::B(SocketTransport { socket, buffers, connections: self.0.clone() })))
    }
}

#[derive(Debug)]
struct SocketTransport {
    socket: Arc<TcpStream>,
    buffers: LazyBuffers,
    connections: Arc<Connections>,
}

impl Transport for SocketTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        if self.connections.stop.is_stopped() {
            return Err(interrupted());
        }
        self.socket.set_write_timeout(timeout.not_zero().map(|d| *d))?;
        let bytes = self.buffers.output().get(..amount).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid HTTP output length."))?;
        (&*self.socket).write_all(bytes).map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ureq::Error::Timeout(timeout.reason),
            _ => error.into(),
        })?;
        Ok(())
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        if self.connections.stop.is_stopped() {
            return Err(interrupted());
        }
        self.socket.set_read_timeout(timeout.not_zero().map(|d| *d))?;
        let read = (&*self.socket).read(self.buffers.input_append_buf()).map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ureq::Error::Timeout(timeout.reason),
            _ => error.into(),
        })?;
        self.buffers.input_appended(read);
        Ok(read > 0)
    }
    fn is_open(&mut self) -> bool {
        if self.connections.stop.is_stopped() || self.socket.set_nonblocking(true).is_err() {
            return false;
        }
        let open = match self.socket.peek(&mut [0; 1]) {
            Ok(size) => size > 0,
            Err(error) => error.kind() == io::ErrorKind::WouldBlock,
        };
        self.socket.set_nonblocking(false).is_ok() && open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_interrupts_a_tls_handshake() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let (ready, waiting) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            assert!(socket.read(&mut [0; 4096]).unwrap() > 0, "the TLS handshake started");
            ready.send(()).unwrap();
            loop {
                if socket.read(&mut [0; 4096]).unwrap() == 0 {
                    break;
                }
            }
        });
        let stop = Stop::default();
        let cancel = stop.clone();
        let (done, finished) = mpsc::channel();
        let client = std::thread::spawn(move || {
            let agent = crate::http::stream_agent_with_stop(None, &stop).unwrap();
            let failed = crate::http::post_json(&agent, "Fixture", &url, &[], &serde_json::json!({})).is_err();
            done.send(failed).unwrap();
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        cancel.stop();
        assert!(finished.recv_timeout(Duration::from_secs(1)).unwrap(), "Stop must close a quiet TLS handshake");
        client.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn stop_interrupts_a_pending_connection_operation() {
        let stop = Stop::default();
        let cancel = stop.clone();
        let (ready, waiting) = mpsc::channel();
        let (release, resumed) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let timeout = NextTimeout { after: Duration::from_secs(3).into(), reason: ureq::Timeout::Connect };
            wait(&stop, timeout, move || {
                ready.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(())
            })
        });
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        cancel.stop();
        assert!(worker.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        // Release the bounded stand-in after observing cancellation.
        release.send(()).unwrap();
    }
}
