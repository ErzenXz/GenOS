//! Bounded A-record decoder for the shell network validation probe.
//! This is not a general resolver or an authenticated DNS interface.

pub fn first_a(response: &[u8]) -> Option<u32> {
    if response.len() < 12
        || response[0..2] != [0x47, 0x45]
        || response[2] & 0x80 == 0
        || response[2] & 0x7a != 0 // Standard query response, complete UDP reply.
        || response[3] & 0x0f != 0 // Error responses cannot prove resolution.
        || u16::from_be_bytes([response[4], response[5]]) != 1
    {
        return None;
    }
    let answers = u16::from_be_bytes([response[6], response[7]]);
    let question_end = skip_dns_name(response, 12)?;
    let mut cursor = question_end.checked_add(4)?;
    if response.get(question_end..cursor)? != [0, 1, 0, 1] {
        return None; // The probe asks for one IN A question.
    }
    for _ in 0..answers {
        cursor = skip_dns_name(response, cursor)?;
        let header_end = cursor
            .checked_add(10)
            .filter(|end| *end <= response.len())?;
        let kind = u16::from_be_bytes([response[cursor], response[cursor + 1]]);
        let class = u16::from_be_bytes([response[cursor + 2], response[cursor + 3]]);
        let data_len = usize::from(u16::from_be_bytes([
            response[cursor + 8],
            response[cursor + 9],
        ]));
        let data_end = header_end
            .checked_add(data_len)
            .filter(|end| *end <= response.len())?;
        if kind == 1 && class == 1 && data_len == 4 {
            return Some(u32::from_be_bytes(
                response[header_end..data_end].try_into().ok()?,
            ));
        }
        cursor = data_end;
    }
    None
}

fn skip_dns_name(packet: &[u8], mut cursor: usize) -> Option<usize> {
    let mut encoded_end = None;
    let mut expanded_length = 1usize; // Include the root terminator.
    for _ in 0..128 {
        let label_start = cursor;
        let length = *packet.get(cursor)?;
        cursor += 1;
        if length == 0 {
            return Some(encoded_end.unwrap_or(cursor));
        }
        if length & 0xc0 == 0xc0 {
            let target = (usize::from(length & 0x3f) << 8) | usize::from(*packet.get(cursor)?);
            // Compression refers to a prior name occurrence. Reject forward,
            // self and out-of-bounds references; the traversal cap also rejects
            // cycles which include an ordinary label before a backward jump.
            if target >= label_start {
                return None;
            }
            encoded_end.get_or_insert(cursor + 1);
            cursor = target;
            continue;
        }
        if length & 0xc0 != 0 || length > 63 {
            return None;
        }
        cursor = cursor
            .checked_add(length as usize)
            .filter(|end| *end <= packet.len())?;
        expanded_length += 1 + usize::from(length);
        if expanded_length > 255 {
            return None;
        }
    }
    None
}

#[cfg(test)]
#[path = "dns_tests.rs"]
mod tests;
