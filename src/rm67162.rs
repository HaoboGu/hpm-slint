use core::arch::asm;

use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::Size;
use embedded_graphics::pixelcolor::raw::ToBytes;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::OriginDimensions;
use embedded_graphics::primitives::Rectangle;
use embedded_graphics::Pixel;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use hpm_hal::mode::Blocking;
use hpm_hal::spi::{AddrLen, AddrPhaseFormat, DataPhaseFormat, Error, Spi, TransMode, TransferConfig};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum Orientation {
    Portrait,
    Landscape,
    PortraitFlipped,
    LandscapeFlipped,
}

impl Orientation {
    pub(crate) fn to_madctr(&self) -> u8 {
        match self {
            Orientation::Portrait => 0x00,
            Orientation::PortraitFlipped => 0b11000000,
            Orientation::Landscape => 0b01100000,
            Orientation::LandscapeFlipped => 0b10100000,
        }
    }
}


pub struct RM67162<'a> {
    qspi: Spi<'a, Blocking>,
    orientation: Orientation,
}

impl RM67162<'_> {
    pub fn new<'a>(qspi: Spi<'a, Blocking>) -> RM67162<'a> {
        RM67162 {
            qspi,
            orientation: Orientation::LandscapeFlipped,
        }
    }

    pub fn set_orientation(&mut self, orientation: Orientation) -> Result<(), Error> {
        self.orientation = orientation;
        self.send_cmd(0x36, &[self.orientation.to_madctr()])
    }

    pub fn reset(&self, rst: &mut impl OutputPin, delay: &mut impl DelayNs) -> Result<(), Error> {
        rst.set_low().unwrap();
        delay.delay_ms(250);

        rst.set_high().unwrap();
        delay.delay_ms(200);
        Ok(())
    }

    /// send 1-1-1 command by default
    fn send_cmd(&mut self, cmd: u32, data: &[u8]) -> Result<(), Error> {
        let mut transfer_config = TransferConfig {
            cmd: Some(0x02),
            addr_len: AddrLen::_24BIT,
            addr: Some(0 | (cmd << 8)),
            addr_phase: AddrPhaseFormat::SINGLE_IO,
            data_phase: DataPhaseFormat::SINGLE_IO,
            transfer_mode: TransMode::WRITE_ONLY,
            dummy_cnt: 0,
            ..Default::default()
        };

        if data.len() == 0 {
            transfer_config.transfer_mode = TransMode::NO_DATA;
            // transfer_config.addr = Some(cmd);
            // transfer_config.addr_len = AddrLen::_16BIT;
            self.qspi.blocking_write::<u8>(&[], &transfer_config)?;
        } else {
            self.qspi.blocking_write(data, &transfer_config)?;
        }

        Ok(())
    }

    fn send_cmd_114(&mut self, cmd: u32, data: &[u8]) -> Result<(), Error> {
        let mut transfer_config = TransferConfig {
            cmd: Some(0x32),
            addr_len: AddrLen::_24BIT,
            addr: Some(0 | (cmd << 8)),
            addr_phase: AddrPhaseFormat::SINGLE_IO,
            data_phase: DataPhaseFormat::QUAD_IO,
            transfer_mode: TransMode::WRITE_ONLY,
            dummy_cnt: 0,
            ..Default::default()
        };

        if data.len() == 0 {
            transfer_config.transfer_mode = TransMode::NO_DATA;
            self.qspi.blocking_write::<u8>(&[], &transfer_config)?;
        } else {
            self.qspi.blocking_write(data, &transfer_config)?;
        }

        Ok(())
    }

    /// rm67162_qspi_init
    pub fn init(&mut self, delay: &mut impl embedded_hal::delay::DelayNs) -> Result<(), Error> {
        defmt::info!("1");
        self.send_cmd(0x11, &[])?; // sleep out
        delay.delay_ms(120);

        defmt::info!("1");
        self.send_cmd(0x3A, &[0x55])?; // 16bit mode

        defmt::info!("1");
        self.send_cmd(0x51, &[0xD0])?; // write brightness

        defmt::info!("1");
        self.send_cmd(0x29, &[])?; // display on
        defmt::info!("wait display n");
        delay.delay_ms(120);

        defmt::info!("display on");
        self.send_cmd(0x51, &[0xD0])?; // write brightness

        defmt::info!("set ori");
        self.set_orientation(self.orientation)?;
        Ok(())
    }

    pub fn set_address(&mut self, x1: u16, y1: u16, x2: u16, y2: u16) -> Result<(), Error> {
        self.send_cmd(
            0x2a,
            &[(x1 >> 8) as u8, (x1 & 0xFF) as u8, (x2 >> 8) as u8, (x2 & 0xFF) as u8],
        )?;
        self.send_cmd(
            0x2b,
            &[(y1 >> 8) as u8, (y1 & 0xFF) as u8, (y2 >> 8) as u8, (y2 & 0xFF) as u8],
        )?;
        self.send_cmd(0x2c, &[])?;
        Ok(())
    }

    pub fn draw_point(&mut self, x: u16, y: u16, color: Rgb565) -> Result<(), Error> {
        self.set_address(x, y, x, y)?;
        self.send_cmd_114(0x2C, &color.to_be_bytes()[..])?;
        // self.send_cmd_114(0x2C, &color.to_le_bytes()[..])?;
        // self.send_cmd_114(0x3C, &color.to_le_bytes()[..])?;
        Ok(())
    }

    pub fn fill_colors(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        mut colors: impl Iterator<Item = Rgb565>,
    ) -> Result<(), Error> {
        self.set_address(x, y, x + w - 1, y + h - 1)?;

        for _ in 1..((w as u32) * (h as u32)) {
            self.send_cmd_114(0x3C, &colors.next().unwrap().to_be_bytes()[..])?;
        }

        Ok(())
    }

    fn fill_color(&mut self, x: u16, y: u16, w: u16, h: u16, color: Rgb565) -> Result<(), Error> {
        self.set_address(x, y, x + w - 1, y + h - 1)?;

        let mut buffer: [u8; 536 * 240] = [0; 536 * 240];
        let total_size = (w as usize) * (h as usize);
        let mut i: usize = 0;
        let mut buffer_idx = 0;
        while i < total_size * 2 {
            if buffer_idx >= buffer.len() {
                i += buffer.len();
                // Write buffer
                self.send_cmd_114(0x3C, &buffer).unwrap();
                buffer_idx = 0;
            }
            if i + buffer_idx >= total_size * 2 {
                break;
            }
            // Fill the buffer
            buffer[buffer_idx] = color.to_be_bytes()[0];
            buffer[buffer_idx + 1] = color.to_be_bytes()[1];
            buffer_idx += 2;
        }

        if buffer_idx > 0 {
            self.send_cmd_114(0x3C, &buffer[..buffer_idx]).unwrap();
        }
        Ok(())
    }

    pub unsafe fn fill_with_framebuffer(&mut self, raw_framebuffer: &[u8]) -> Result<(), Error> {
        self.set_address(0, 0, self.size().width as u16 - 1, self.size().height as u16 - 1)?;

        self.send_cmd_114(0x3C, raw_framebuffer)?;

        Ok(())
    }
}

impl OriginDimensions for RM67162<'_> {
    fn size(&self) -> Size {
        if matches!(self.orientation, Orientation::Landscape | Orientation::LandscapeFlipped) {
            Size::new(536, 240)
        } else {
            Size::new(240, 536)
        }
    }
}

impl DrawTarget for RM67162<'_> {
    type Color = Rgb565;

    type Error = Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = embedded_graphics::Pixel<Self::Color>>,
    {
        for Pixel(pt, color) in pixels {
            if pt.x < 0 || pt.y < 0 {
                continue;
            }
            self.draw_point(pt.x as u16, pt.y as u16, color)?;
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        self.fill_color(
            area.top_left.x as u16,
            area.top_left.y as u16,
            area.size.width as u16,
            area.size.height as u16,
            color,
        )?;
        Ok(())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        self.fill_colors(
            area.top_left.x as u16,
            area.top_left.y as u16,
            area.size.width as u16,
            area.size.height as u16,
            colors.into_iter(),
        )?;
        Ok(())
    }
}