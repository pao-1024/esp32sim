//! ESP32-S3 non-DMA RX channels 4..7. Register fields and signal numbers follow
//! ESP-IDF v6.0.1 soc/esp32s3/register/soc/rmt_reg.h and esp_hal_rmt/esp32s3/include/hal/rmt_ll.h.
// shortcut: non-DMA bounded captures omit wrapping, filtering and carrier removal; extend when a driver needs those modes.
use super::RMT_MEM_WORDS;

#[derive(Clone)]
pub(super) struct Channel {
    conf0: u32, conf1: u32, limit: u32,
    started: Option<u64>, last: u64, level: bool,
    halves: usize,
}
impl Default for Channel {
    fn default() -> Self { Self { conf0: 0x0100_0002 | (32767 << 8), conf1: 8 | (15 << 5), limit: 0, started: None, last: 0, level: false, halves: 0 } }
}
#[derive(Default)]
pub(super) struct Receive { channels: [Channel; 4] }
impl Receive {
    pub fn active(&self) -> bool { self.channels.iter().any(|c| c.conf1 & 1 != 0) }
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            0x30..=0x4c => { let c = &self.channels[((off - 0x30) / 8) as usize]; Some(if off & 4 == 0 { c.conf0 } else { c.conf1 }) }
            0x60..=0x6c => {
                let n = ((off - 0x60) / 4) as usize;
                let c = &self.channels[n];
                Some(((n + 4) * RMT_MEM_WORDS + c.halves.div_ceil(2)) as u32 | if c.conf1 & 1 != 0 { 1 << 22 } else { 0 })
            }
            0xb0..=0xbc => Some(self.channels[((off - 0xb0) / 4) as usize].limit),
            _ => None,
        }
    }
    pub fn write(&mut self, off: u32, value: u32) {
        match off {
            0x30..=0x4c => {
                let c = &mut self.channels[((off - 0x30) / 8) as usize];
                if off & 4 == 0 { c.conf0 = value; }
                else {
                    if value & 6 != 0 { c.halves = 0; }
                    if (value ^ c.conf1) & 1 != 0 { c.started = None; }
                    // Pointer resets, FIFO reset and CONF_UPDATE are commands.
                    c.conf1 = value & !(6 | (1 << 14) | (1 << 15));
                }
            }
            0xb0..=0xbc => self.channels[((off - 0xb0) / 4) as usize].limit = value,
            _ => {}
        }
    }
    fn ticks_per_cycle(conf0: u32, sys: u32, cpu_hz: u64) -> f64 {
        let source = match (sys >> 24) & 3 { 2 => 8_000_000.0, 3 => 40_000_000.0, _ => 80_000_000.0 };
        let numerator = ((sys >> 12) & 63) as f64;
        let denominator = ((sys >> 18) & 63).max(1) as f64;
        let group_div = 1.0 + ((sys >> 4) & 255) as f64 + numerator / denominator;
        let divider = if conf0 & 255 == 0 { 256 } else { conf0 & 255 };
        source / group_div / divider as f64 / cpu_hz as f64
    }
    fn put(c: &mut Channel, n: usize, duration: u32, level: bool, mem: &mut [u32], irq: &mut u32) {
        let start = (n + 4) * RMT_MEM_WORDS;
        let capacity = (((c.conf0 >> 24) & 15) as usize * RMT_MEM_WORDS).min(mem.len() - start);
        if c.halves / 2 >= capacity {
            c.conf1 &= !1;
            *irq |= 1 << (20 + n);
            return;
        }
        let index = start + c.halves / 2;
        if c.halves & 1 == 0 { mem[index] = 0; }
        mem[index] |= (duration.min(32767) | ((level as u32) << 15)) << ((c.halves & 1) * 16);
        c.halves += 1;
    }
    fn advance_channel(c: &mut Channel, n: usize, cycle: u64, rate: f64, mem: &mut [u32], irq: &mut u32) {
        if c.conf1 & 1 == 0 { return; }
        if c.started.is_none() { c.started = Some(cycle); c.last = cycle; }
        let idle = ((c.conf0 >> 8) & 32767).max(1);
        if cycle.saturating_sub(c.last) as f64 * rate >= idle as f64 {
            Self::put(c, n, idle, c.level, mem, irq);
            // A zero-duration half terminates a receive, including an unmatched final half.
            if c.halves & 1 == 0 { Self::put(c, n, 0, false, mem, irq); }
            c.conf1 &= !1;
            *irq |= 1 << (16 + n);
        }
    }
    pub fn advance(&mut self, cycle: u64, sys: u32, cpu_hz: u64, mem: &mut [u32], irq: &mut u32) {
        for (n, c) in self.channels.iter_mut().enumerate() {
            Self::advance_channel(c, n, cycle, Self::ticks_per_cycle(c.conf0, sys, cpu_hz), mem, irq);
        }
    }
    pub fn input(&mut self, n: usize, cycle: u64, level: bool, clock: (u32, u64), mem: &mut [u32], irq: &mut u32) {
        let Some(c) = self.channels.get_mut(n) else { return; };
        let rate = Self::ticks_per_cycle(c.conf0, clock.0, clock.1);
        Self::advance_channel(c, n, cycle, rate, mem, irq);
        if c.conf1 & 1 == 0 || c.level == level { return; }
        let duration = (cycle.saturating_sub(c.last) as f64 * rate).round() as u32;
        if duration != 0 { Self::put(c, n, duration, c.level, mem, irq); }
        c.last = cycle;
        c.level = level;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_input_duration_idle_end_and_rearm() {
        let mut rx = Receive::default();
        let mut mem = [0; 384];
        let mut irq = 0;
        let sys = 1 << 24; // APB, no group divider.
        rx.write(0x30, 80 | (30000 << 8) | (2 << 24));
        rx.write(0x34, 1 | 2 | 8);
        rx.advance(0, sys, 240_000_000, &mut mem, &mut irq);
        rx.input(0, 200 * 240, true, (sys, 240_000_000), &mut mem, &mut irq);
        rx.input(0, 6000 * 240, false, (sys, 240_000_000), &mut mem, &mut irq);
        rx.advance(36000 * 240, sys, 240_000_000, &mut mem, &mut irq);
        assert_eq!(mem[192], 200 | ((5800 | 0x8000) << 16));
        assert_eq!(mem[193], 30000);
        assert_eq!(rx.read(0x60), Some(194));
        assert_eq!(irq, 1 << 16);
        assert!(!rx.active());
        rx.write(0x34, 1 | 2 | 8);
        rx.advance(40000 * 240, sys, 240_000_000, &mut mem, &mut irq);
        rx.advance(70000 * 240, sys, 240_000_000, &mut mem, &mut irq);
        assert_eq!(mem[192], 30000, "no echo is an all-low capture");
        assert_eq!(rx.read(0x60), Some(193));
        rx.write(0x34, 0);
        rx.input(0, 71000 * 240, true, (sys, 240_000_000), &mut mem, &mut irq);
        assert_eq!(rx.read(0x60), Some(193), "disabled channels ignore input");
    }
}
