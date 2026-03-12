//! UDP networking operations for the Coral runtime (L3.3).
//!
//! Provides `coral_udp_bind`, `coral_udp_send`, and `coral_udp_recv` FFI
//! functions using standard library UDP sockets.

use crate::*;
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::Mutex;

static UDP_SOCKETS: std::sync::LazyLock<Mutex<HashMap<u64, UdpSocket>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static UDP_NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// `coral_udp_bind(host, port) -> handle_number | -1`
///
/// Bind a UDP socket to host:port. Returns a handle number on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_udp_bind(host_handle: ValueHandle, port_handle: ValueHandle) -> ValueHandle {
    let host = match safe_string(host_handle) {
        Some(s) => s,
        None => return coral_make_number(-1.0),
    };
    let port = safe_number(port_handle) as u16;
    let addr = format!("{}:{}", host, port);
    match UdpSocket::bind(&addr) {
        Ok(sock) => {
            let id = UDP_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            UDP_SOCKETS.lock().unwrap().insert(id, sock);
            coral_make_number(id as f64)
        }
        Err(_) => coral_make_number(-1.0),
    }
}

/// `coral_udp_send(handle, data, dest_host, dest_port) -> bytes_sent | -1`
///
/// Send data to a destination address via a bound UDP socket.
#[unsafe(no_mangle)]
pub extern "C" fn coral_udp_send(
    handle: ValueHandle,
    data_handle: ValueHandle,
    dest_host_handle: ValueHandle,
    dest_port_handle: ValueHandle,
) -> ValueHandle {
    let id = safe_number(handle) as u64;
    let data = match safe_string(data_handle) {
        Some(s) => s,
        None => return coral_make_number(-1.0),
    };
    let dest_host = match safe_string(dest_host_handle) {
        Some(s) => s,
        None => return coral_make_number(-1.0),
    };
    let dest_port = safe_number(dest_port_handle) as u16;
    let dest = format!("{}:{}", dest_host, dest_port);

    let sockets = UDP_SOCKETS.lock().unwrap();
    match sockets.get(&id) {
        Some(sock) => match sock.send_to(data.as_bytes(), &dest) {
            Ok(n) => coral_make_number(n as f64),
            Err(_) => coral_make_number(-1.0),
        },
        None => coral_make_number(-1.0),
    }
}

/// `coral_udp_recv(handle, max_bytes) -> map {"data": str, "addr": str} | -1`
///
/// Receive data from a bound UDP socket. Blocks until data arrives.
/// Returns a map with "data" and "addr" keys on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_udp_recv(handle: ValueHandle, max_bytes_handle: ValueHandle) -> ValueHandle {
    let id = safe_number(handle) as u64;
    let max_bytes = safe_number(max_bytes_handle) as usize;
    let max_bytes = max_bytes.min(65536); // cap at 64KB

    let sockets = UDP_SOCKETS.lock().unwrap();
    match sockets.get(&id) {
        Some(sock) => {
            let mut buf = vec![0u8; max_bytes];
            // Drop the lock before blocking recv
            let sock_clone = match sock.try_clone() {
                Ok(s) => s,
                Err(_) => return coral_make_number(-1.0),
            };
            drop(sockets);

            match sock_clone.recv_from(&mut buf) {
                Ok((n, addr)) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let entries = vec![
                        MapEntry {
                            key: coral_make_string_from_rust("data"),
                            value: coral_make_string_from_rust(&data),
                        },
                        MapEntry {
                            key: coral_make_string_from_rust("addr"),
                            value: coral_make_string_from_rust(&addr.to_string()),
                        },
                    ];
                    coral_make_map(entries.as_ptr(), entries.len())
                }
                Err(_) => coral_make_number(-1.0),
            }
        }
        None => coral_make_number(-1.0),
    }
}

/// `coral_udp_close(handle) -> 0`
///
/// Close a UDP socket.
#[unsafe(no_mangle)]
pub extern "C" fn coral_udp_close(handle: ValueHandle) -> ValueHandle {
    let id = safe_number(handle) as u64;
    UDP_SOCKETS.lock().unwrap().remove(&id);
    coral_make_number(0.0)
}

fn safe_string(h: ValueHandle) -> Option<String> {
    if h.is_null() {
        return None;
    }
    let val = unsafe { &*h };
    if val.tag != ValueTag::String as u8 {
        return None;
    }
    Some(value_to_rust_string(val))
}

fn safe_number(h: ValueHandle) -> f64 {
    if h.is_null() {
        return 0.0;
    }
    let val = unsafe { &*h };
    if val.tag != ValueTag::Number as u8 {
        return 0.0;
    }
    unsafe { val.payload.number }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_bind_and_close() {
        let host = coral_make_string_from_rust("127.0.0.1");
        let port = coral_make_number(0.0); // OS-assigned port
        let handle = coral_udp_bind(host, port);
        let id = unsafe { (&*handle).payload.number };
        assert!(id > 0.0, "bind should return positive handle");

        let result = coral_udp_close(handle);
        let val = unsafe { (&*result).payload.number };
        assert_eq!(val, 0.0);
    }

    #[test]
    fn udp_send_recv_loopback() {
        // Bind two sockets on loopback
        let host = coral_make_string_from_rust("127.0.0.1");
        let port_zero = coral_make_number(0.0);

        let s1 = coral_udp_bind(host, port_zero);
        let s1_id = unsafe { (*s1).payload.number };
        assert!(s1_id > 0.0);

        let host2 = coral_make_string_from_rust("127.0.0.1");
        let s2 = coral_udp_bind(host2, port_zero);
        let s2_id = unsafe { (*s2).payload.number };
        assert!(s2_id > 0.0);

        // Get s2's actual port from the socket
        let s2_port = {
            let sockets = UDP_SOCKETS.lock().unwrap();
            sockets.get(&(s2_id as u64)).unwrap().local_addr().unwrap().port()
        };

        // Send from s1 to s2
        let data = coral_make_string_from_rust("hello");
        let dest_host = coral_make_string_from_rust("127.0.0.1");
        let dest_port = coral_make_number(s2_port as f64);
        let sent = coral_udp_send(s1, data, dest_host, dest_port);
        let sent_n = unsafe { (&*sent).payload.number };
        assert_eq!(sent_n, 5.0);

        // Recv on s2
        let max = coral_make_number(1024.0);
        let result = coral_udp_recv(s2, max);
        let result_ref = unsafe { &*result };
        assert_eq!(result_ref.tag, ValueTag::Map as u8);

        coral_udp_close(s1);
        coral_udp_close(s2);
    }

    #[test]
    fn udp_invalid_handle_returns_error() {
        let bad_handle = coral_make_number(99999.0);
        let data = coral_make_string_from_rust("test");
        let host = coral_make_string_from_rust("127.0.0.1");
        let port = coral_make_number(1234.0);
        let result = coral_udp_send(bad_handle, data, host, port);
        let val = unsafe { (&*result).payload.number };
        assert_eq!(val, -1.0);
    }
}
