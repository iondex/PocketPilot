//! Bounded reassembly of SDK protobuf event fragments.
use super::headers;
use lark_websocket_protobuf::pbbp2::Frame;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const MAX_PARTS: usize = 32;
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
const MAX_PENDING: usize = 64;
const MAX_TOTAL: usize = 16 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) struct FramePackageBuffer {
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
    created: Instant,
}

pub(crate) fn assemble_frame(
    buffers: &mut HashMap<String, FramePackageBuffer>,
    mut frame: Frame,
) -> Option<Frame> {
    buffers.retain(|_, b| b.created.elapsed() < TTL);
    if frame.headers.len() > 64
        || frame
            .headers
            .iter()
            .any(|h| h.key.len() > 128 || h.value.len() > 1024)
    {
        return None;
    }
    let headers = frame.headers.as_ref();
    let sum = match headers::header_value(headers, headers::HDR_SUM) {
        Some(v) => v.parse::<usize>().ok()?,
        None => 1,
    };
    let seq = match headers::header_value(headers, headers::HDR_SEQ) {
        Some(v) => v.parse::<usize>().ok()?,
        None => 0,
    };
    let id = headers::header_value(headers, headers::HDR_MESSAGE_ID)
        .unwrap_or("")
        .to_owned();
    let payload = frame.payload.take()?;
    let sum = if sum == 0 { 1 } else { sum };
    if sum > MAX_PARTS || seq >= sum || payload.len() > MAX_MESSAGE || id.len() > 512 {
        return None;
    }
    if sum == 1 {
        frame.payload = Some(payload);
        return Some(frame);
    }
    if id.is_empty() {
        return None;
    }
    if let Some(existing) = buffers.get(&id) {
        if existing.parts.len() != sum {
            buffers.remove(&id);
            return None;
        }
        if let Some(previous) = &existing.parts[seq] {
            if previous != &payload {
                buffers.remove(&id);
            }
            return None;
        }
        if existing.bytes.saturating_add(payload.len()) > MAX_MESSAGE {
            buffers.remove(&id);
            return None;
        }
    } else if buffers.len() >= MAX_PENDING {
        return None;
    }
    let total: usize = buffers.values().map(|b| b.bytes).sum();
    if total.saturating_add(payload.len()) > MAX_TOTAL {
        return None;
    }
    let buffer = buffers
        .entry(id.clone())
        .or_insert_with(|| FramePackageBuffer {
            parts: vec![None; sum],
            bytes: 0,
            created: Instant::now(),
        });
    buffer.bytes += payload.len();
    buffer.parts[seq] = Some(payload);
    if buffer.parts.iter().any(Option::is_none) {
        return None;
    }
    let buffer = buffers.remove(&id)?;
    let mut output = Vec::with_capacity(buffer.bytes);
    for part in buffer.parts.into_iter().flatten() {
        output.extend(part);
    }
    frame.payload = Some(output);
    Some(frame)
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
