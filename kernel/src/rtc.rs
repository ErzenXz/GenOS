use kernel::display::FixedText;

use crate::arch;

const CMOS_ADDRESS: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

#[derive(Clone, Copy)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl RtcTime {
    pub fn format_clock(self) -> FixedText {
        let mut text = FixedText::empty();
        push_two(&mut text, self.hour);
        text.push_str(":");
        push_two(&mut text, self.minute);
        text.push_str(":");
        push_two(&mut text, self.second);
        text
    }

    pub fn format_date_time(self) -> FixedText {
        let mut text = FixedText::from_str("rtc ");
        text.push_u64(self.year as u64);
        text.push_str("-");
        push_two(&mut text, self.month);
        text.push_str("-");
        push_two(&mut text, self.day);
        text.push_str(" ");
        push_two(&mut text, self.hour);
        text.push_str(":");
        push_two(&mut text, self.minute);
        text.push_str(":");
        push_two(&mut text, self.second);
        text
    }
}

pub fn read() -> RtcTime {
    let mut second = read_register(0x00);
    let mut minute = read_register(0x02);
    let mut hour = read_register(0x04);
    let mut day = read_register(0x07);
    let mut month = read_register(0x08);
    let mut year = read_register(0x09);
    let status_b = read_register(0x0b);

    if status_b & 0x04 == 0 {
        second = bcd_to_binary(second);
        minute = bcd_to_binary(minute);
        hour = bcd_to_binary(hour & 0x7f) | (hour & 0x80);
        day = bcd_to_binary(day);
        month = bcd_to_binary(month);
        year = bcd_to_binary(year);
    }

    if status_b & 0x02 == 0 {
        let pm = hour & 0x80 != 0;
        hour &= 0x7f;
        if pm && hour < 12 {
            hour += 12;
        }
        if !pm && hour == 12 {
            hour = 0;
        }
    }

    RtcTime {
        year: 2000 + year as u16,
        month,
        day,
        hour,
        minute,
        second,
    }
}

fn read_register(register: u8) -> u8 {
    unsafe {
        arch::outb(CMOS_ADDRESS, register);
        arch::inb(CMOS_DATA)
    }
}

fn bcd_to_binary(value: u8) -> u8 {
    (value & 0x0f) + ((value / 16) * 10)
}

fn push_two(text: &mut FixedText, value: u8) {
    text.push_str(core::str::from_utf8(&[b'0' + (value / 10)]).unwrap_or(""));
    text.push_str(core::str::from_utf8(&[b'0' + (value % 10)]).unwrap_or(""));
}
