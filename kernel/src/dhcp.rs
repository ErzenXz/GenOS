//! Bounded DHCP wire codec used by the production network coordinator.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpConfig {
    pub address: [u8; 4],
    pub subnet: [u8; 4],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub server: [u8; 4],
}

pub fn build_dhcp(
    output: &mut [u8; 300],
    mac: [u8; 6],
    xid: u32,
    message_type: u8,
    requested: Option<[u8; 4]>,
    server: Option<[u8; 4]>,
) -> Option<usize> {
    output.fill(0);
    output[0] = 1;
    output[1] = 1;
    output[2] = 6;
    output[4..8].copy_from_slice(&xid.to_be_bytes());
    output[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    output[28..34].copy_from_slice(&mac);
    output[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut cursor = 240;
    push_option(output, &mut cursor, 53, &[message_type])?;
    if let Some(address) = requested {
        push_option(output, &mut cursor, 50, &address)?;
    }
    if let Some(address) = server {
        push_option(output, &mut cursor, 54, &address)?;
    }
    push_option(output, &mut cursor, 55, &[1, 3, 6])?;
    *output.get_mut(cursor)? = 255;
    Some(cursor + 1)
}

fn push_option(output: &mut [u8], cursor: &mut usize, kind: u8, value: &[u8]) -> Option<()> {
    let end = cursor.checked_add(2 + value.len())?;
    if end > output.len() || value.len() > u8::MAX as usize {
        return None;
    }
    output[*cursor] = kind;
    output[*cursor + 1] = value.len() as u8;
    output[*cursor + 2..end].copy_from_slice(value);
    *cursor = end;
    Some(())
}

pub fn parse_dhcp(payload: &[u8], xid: u32, expected_type: u8) -> Option<DhcpConfig> {
    if payload.len() < 241
        || payload[0] != 2
        || payload[4..8] != xid.to_be_bytes()
        || payload[236..240] != [99, 130, 83, 99]
    {
        return None;
    }
    let address = payload[16..20].try_into().ok()?;
    let mut config = DhcpConfig {
        address,
        subnet: [255, 255, 255, 0],
        gateway: [0; 4],
        dns: [0; 4],
        server: [0; 4],
    };
    let mut message_type = 0;
    let mut cursor = 240;
    while cursor < payload.len() {
        let kind = payload[cursor];
        cursor += 1;
        if kind == 255 {
            break;
        }
        if kind == 0 {
            continue;
        }
        let len = *payload.get(cursor)? as usize;
        cursor += 1;
        let end = cursor
            .checked_add(len)
            .filter(|end| *end <= payload.len())?;
        let value = &payload[cursor..end];
        match (kind, value) {
            (53, [value]) => message_type = *value,
            (1, [a, b, c, d]) => config.subnet = [*a, *b, *c, *d],
            (3, [a, b, c, d, ..]) => config.gateway = [*a, *b, *c, *d],
            (6, [a, b, c, d, ..]) => config.dns = [*a, *b, *c, *d],
            (54, [a, b, c, d]) => config.server = [*a, *b, *c, *d],
            _ => {}
        }
        cursor = end;
    }
    (message_type == expected_type
        && config.address != [0; 4]
        && config.gateway != [0; 4]
        && config.dns != [0; 4]
        && config.server != [0; 4])
        .then_some(config)
}
