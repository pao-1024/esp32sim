use crate::periph::Peripherals;
use esp_soc::{board::SpiPins, pins::ChipPins};

impl Peripherals {
    // ESP-IDF v6.0.1 esp32s3 gpio_sig_map.h: RMT_SIG_IN0..3 = 81..84;
    // GPIO matrix IN_SEL bit 7, IN_INV bit 6, pin bits 0..5 (gpio_reg.h).
    pub(crate) fn rmt_input(&mut self, pin: u8, level: bool, cycle: u64) {
        if !self.rmt.rx_active() { return; }
        for channel in 0..4 {
            let sel = self.gpio.func_in_sel[81 + channel];
            if sel & 0x80 != 0 && sel & 0x3f == pin as u32 && ChipPins::S3.routes(&self.gpio, &self.io_mux).input_function(pin as usize, 1) {
                self.rmt.rx_input(channel, cycle, level ^ (sel & 0x40 != 0));
            }
        }
    }
    pub(crate) fn i2c_pin(&self, signal: usize) -> Option<u8> { ChipPins::S3.routes(&self.gpio, &self.io_mux).i2c_pin(signal) }
    pub(crate) fn spi2_pins(&self) -> SpiPins {
        ChipPins::S3.routes(&self.gpio, &self.io_mux).spi_pins(&self.spi2, [101, 102, 103, 110, 111, 112, 113, 114, 115], 256, &[(4, 12, 11, 13, &[10]), (2, 36, 35, 37, &[34])])
    }
    /// SPI3 has no IO_MUX pins on the S3, only the matrix: SPI3_CLK 66, SPI3_Q 67, SPI3_D 68,
    /// SPI3_CS0..2 71, 72, 127 (IDF v5.5.5 components/soc/esp32s3/include/soc/gpio_sig_map.h:136-147,
    /// :226; v4.4.8 :144-155, :234 the same). The controller's CS3..5 enables select nothing.
    pub(crate) fn spi3_pins(&self) -> SpiPins {
        const NO_SIGNAL: u32 = u32::MAX;
        ChipPins::S3.routes(&self.gpio, &self.io_mux).spi_pins(&self.spi3, [66, 67, 68, 71, 72, 127, NO_SIGNAL, NO_SIGNAL, NO_SIGNAL], 256, &[])
    }
    /// GP-SPI `host` 2 (SPI2, FSPI) or 3 (SPI3, HSPI).
    pub fn gpspi(&self, host: u8) -> &esp_periph::GpSpi { if host == 3 { &self.spi3 } else { &self.spi2 } }
    pub fn gpspi_mut(&mut self, host: u8) -> &mut esp_periph::GpSpi { if host == 3 { &mut self.spi3 } else { &mut self.spi2 } }
}

impl crate::bus::SocBus {
    pub(crate) fn drain_routed_board_inputs(&mut self) {
        for edge in self.board.take_edges() {
            self.irq_dirty |= self.periph.gpio.set_input(edge.pin, edge.level);
            self.periph.rmt_input(edge.pin, edge.level, edge.cycle);
            if let Some(events) = &mut self.gpio_events { events.push((edge.cycle, edge.pin, edge.level)); }
        }
        for pin in self.board.released_inputs() {
            self.irq_dirty |= esp_soc::gpio::release_and_report(&mut self.periph.gpio, &mut self.gpio_events, self.cycles, pin);
            self.periph.rmt_input(pin, self.periph.gpio.level(pin), self.cycles);
        }
        if self.periph.rmt.rx_active() {
            self.periph.rmt.rx_advance(self.cycles);
            self.irq_dirty |= self.periph.rmt.irq();
        }
    }
}
