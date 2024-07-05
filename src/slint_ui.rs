//! Slint interface
//!

extern crate alloc;
use alloc::rc::Rc;

use embedded_graphics_core::pixelcolor::raw::RawU16;
use embedded_graphics_core::prelude::*;
use embedded_graphics_core::primitives::Rectangle;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::Platform;

slint::include_modules!();

pub struct MyPlatform {
    pub window: Rc<MinimalSoftwareWindow>,
}

impl Platform for MyPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        // Since on MCUs, there can be only one window, just return a clone of self.window.
        // We'll also use the same window in the event loop.
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_micros(embassy_time::Instant::now().as_micros())
    }

    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        Ok(())
    }
}

pub struct DisplayWrapper<'a, T> {
    pub display: &'a mut T,
    pub line_buffer: &'a mut [slint::platform::software_renderer::Rgb565Pixel],
}

impl<T: DrawTarget<Color = embedded_graphics_core::pixelcolor::Rgb565>>
    slint::platform::software_renderer::LineBufferProvider for DisplayWrapper<'_, T>
{
    type TargetPixel = slint::platform::software_renderer::Rgb565Pixel;
    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        // Render into the line
        render_fn(&mut self.line_buffer[range.clone()]);

        // Send the line to the screen using DrawTarget::fill_contiguous
        self.display
            .fill_contiguous(
                &Rectangle::new(Point::new(range.start as _, line as _), Size::new(range.len() as _, 1)),
                self.line_buffer[range.clone()].iter().map(|p| RawU16::new(p.0).into()),
            )
            .map_err(drop)
            .unwrap();
    }
}
