use crate::actor::{ActorHandle, ActorSystem, Message};
use crate::nanbox::NanBoxedValue;
use crate::{Payload, Value, ValueHandle, ValueTag};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;

const MSG_TYPE_SEND: u8 = 2;

const WIRE_TAG_NUMBER: u8 = 0;
const WIRE_TAG_BOOL: u8 = 1;
const WIRE_TAG_STRING: u8 = 2;
const WIRE_TAG_UNIT: u8 = 7;

fn serialize_nanboxed(nb: NanBoxedValue) -> Vec<u8> {
    if nb.is_number() {
        let mut buf = vec![WIRE_TAG_NUMBER];
        buf.extend_from_slice(&nb.as_number().to_le_bytes());
        buf
    } else if nb.is_bool() {
        vec![WIRE_TAG_BOOL, if nb.as_bool() { 1 } else { 0 }]
    } else if nb.is_unit() {
        vec![WIRE_TAG_UNIT]
    } else if nb.is_heap_ptr() {
        let ptr = nb.as_heap_ptr();
        let val = unsafe { &*ptr };
        if val.tag == ValueTag::String as u8 {
            let s = unsafe { value_to_string(val) };
            let bytes = s.as_bytes();
            let mut buf = vec![WIRE_TAG_STRING];
            buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(bytes);
            buf
        } else {
            let mut buf = vec![WIRE_TAG_NUMBER];
            buf.extend_from_slice(&0.0f64.to_le_bytes());
            buf
        }
    } else {
        vec![WIRE_TAG_UNIT]
    }
}

unsafe fn value_to_string(val: &Value) -> String {
    let payload_bytes: &[u8; 40] = unsafe { &val.payload.inline };
    let len = payload_bytes[0] as usize;
    if len <= 39 {
        String::from_utf8_lossy(&payload_bytes[1..1 + len]).to_string()
    } else {
        let ptr = unsafe { val.payload.ptr } as *const u8;
        let actual_len = unsafe { *(ptr.sub(8) as *const u64) } as usize;
        let slice = unsafe { std::slice::from_raw_parts(ptr, actual_len) };
        String::from_utf8_lossy(slice).to_string()
    }
}

fn deserialize_to_handle(data: &[u8]) -> Option<(ValueHandle, usize)> {
    if data.is_empty() {
        return None;
    }
    match data[0] {
        WIRE_TAG_NUMBER => {
            if data.len() < 9 {
                return None;
            }
            let arr: [u8; 8] = data[1..9].try_into().ok()?;
            let f = f64::from_le_bytes(arr);
            let nb = NanBoxedValue::from_number(f);
            Some((nb.to_bits() as ValueHandle, 9))
        }
        WIRE_TAG_BOOL => {
            if data.len() < 2 {
                return None;
            }
            let nb = NanBoxedValue::from_bool(data[1] != 0);
            Some((nb.to_bits() as ValueHandle, 2))
        }
        WIRE_TAG_STRING => {
            if data.len() < 5 {
                return None;
            }
            let len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize;
            if data.len() < 5 + len {
                return None;
            }
            let s = &data[5..5 + len];
            let handle = crate::nanbox_ffi::coral_nb_make_string(s.as_ptr(), s.len());
            Some((handle as ValueHandle, 5 + len))
        }
        WIRE_TAG_UNIT => {
            let nb = NanBoxedValue::unit();
            Some((nb.to_bits() as ValueHandle, 1))
        }
        _ => None,
    }
}

fn write_frame(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&[msg_type])?;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let msg_type = header[0];
    let len = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut payload = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok((msg_type, payload))
}

pub struct RemoteNode {
    listen_addr: String,
    system: ActorSystem,
    peers: Arc<Mutex<HashMap<String, TcpStream>>>,
}

impl RemoteNode {
    pub fn new(listen_addr: &str, system: ActorSystem) -> Self {
        Self {
            listen_addr: listen_addr.to_string(),
            system,
            peers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_listener(&self) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.listen_addr)?;
        let system = self.system.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(stream) = stream {
                    let sys = system.clone();
                    thread::spawn(move || {
                        handle_remote_connection(stream, sys);
                    });
                }
            }
        });
        Ok(())
    }

    pub fn connect_peer(&self, addr: &str) -> std::io::Result<()> {
        let stream = TcpStream::connect(addr)?;
        self.peers.lock().unwrap().insert(addr.to_string(), stream);
        Ok(())
    }

    pub fn send_remote(&self, peer_addr: &str, actor_name: &str, nb_value: NanBoxedValue) -> bool {
        let mut peers = self.peers.lock().unwrap();
        if let Some(stream) = peers.get_mut(peer_addr) {
            let mut payload = Vec::new();
            let name_bytes = actor_name.as_bytes();
            payload.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            payload.extend_from_slice(name_bytes);
            payload.extend_from_slice(&serialize_nanboxed(nb_value));
            write_frame(stream, MSG_TYPE_SEND, &payload).is_ok()
        } else {
            false
        }
    }
}

fn handle_remote_connection(mut stream: TcpStream, system: ActorSystem) {
    loop {
        match read_frame(&mut stream) {
            Ok((MSG_TYPE_SEND, payload)) => {
                if payload.len() < 4 {
                    continue;
                }
                let name_len =
                    u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                if payload.len() < 4 + name_len + 1 {
                    continue;
                }
                let name = String::from_utf8_lossy(&payload[4..4 + name_len]).to_string();
                if let Some((handle_val, _)) = deserialize_to_handle(&payload[4 + name_len..]) {
                    if let Some(actor_handle) = system.lookup_named(&name) {
                        let _ = system.send(&actor_handle, Message::User(handle_val));
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

pub struct RemoteProxy {
    peer_addr: String,
    actor_name: String,
    node: Arc<RemoteNode>,
}

impl RemoteProxy {
    pub fn new(node: Arc<RemoteNode>, peer_addr: &str, actor_name: &str) -> Self {
        Self {
            peer_addr: peer_addr.to_string(),
            actor_name: actor_name.to_string(),
            node,
        }
    }

    pub fn send(&self, nb_value: NanBoxedValue) -> bool {
        self.node
            .send_remote(&self.peer_addr, &self.actor_name, nb_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_number_roundtrip() {
        let nb = NanBoxedValue::from_number(42.5);
        let bytes = serialize_nanboxed(nb);
        assert_eq!(bytes[0], WIRE_TAG_NUMBER);
        let (handle, consumed) = deserialize_to_handle(&bytes).unwrap();
        assert_eq!(consumed, 9);
        let recovered = NanBoxedValue::from_bits(handle as u64);
        assert_eq!(recovered.as_number(), 42.5);
    }

    #[test]
    fn serialize_bool_roundtrip() {
        let nb = NanBoxedValue::from_bool(true);
        let bytes = serialize_nanboxed(nb);
        assert_eq!(bytes[0], WIRE_TAG_BOOL);
        let (handle, consumed) = deserialize_to_handle(&bytes).unwrap();
        assert_eq!(consumed, 2);
        let recovered = NanBoxedValue::from_bits(handle as u64);
        assert!(recovered.as_bool());
    }

    #[test]
    fn serialize_unit_roundtrip() {
        let nb = NanBoxedValue::unit();
        let bytes = serialize_nanboxed(nb);
        assert_eq!(bytes[0], WIRE_TAG_UNIT);
        let (handle, consumed) = deserialize_to_handle(&bytes).unwrap();
        assert_eq!(consumed, 1);
        let recovered = NanBoxedValue::from_bits(handle as u64);
        assert!(recovered.is_unit());
    }

    #[test]
    fn frame_encoding() {
        let payload = b"hello world";
        let mut buf = Vec::new();
        buf.push(MSG_TYPE_SEND);
        let len = payload.len() as u32;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(payload);

        assert_eq!(buf[0], MSG_TYPE_SEND);
        let recovered_len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        assert_eq!(recovered_len, payload.len());
        assert_eq!(&buf[5..], payload);
    }
}
