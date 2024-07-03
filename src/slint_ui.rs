//! Slint interface
//!

// extern crate alloc;
use core::cell::RefCell;

use alloc::rc::Rc;

use embedded_graphics_core::pixelcolor::raw::RawU16;
use embedded_graphics_core::prelude::*;
use embedded_graphics_core::primitives::Rectangle;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::Platform;

use crate::rm67162::RM67162;

slint::include_modules!();

pub struct MyPlatform<'a> {
    pub window: Rc<MinimalSoftwareWindow>,
    pub display: RefCell<RM67162<'a>>,
    pub line_buffer: RefCell<[slint::platform::software_renderer::Rgb565Pixel; 536]>,
}

impl<'a> Platform for MyPlatform<'a> {
    fn create_window_adapter(&self) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        // Since on MCUs, there can be only one window, just return a clone of self.window.
        // We'll also use the same window in the event loop.
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_micros(embassy_time::Instant::now().as_micros())
    }
    // optional: You can put the event loop there, or in the main function, see later
    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        loop {
            // Let Slint run the timer hooks and update animations.
            slint::platform::update_timers_and_animations();

            // Check the touch screen or input device using your driver.
            // TODO: touch driver
            // if let Some(event) = check_for_touch_event(/*...*/) {
            //     // convert the event from the driver into a `slint::platform::WindowEvent`
            //     // and pass it to the window.
            //     window.dispatch_event(event);
            // }

            // Draw the scene if something needs to be drawn.
            self.window.draw_if_needed(|renderer| {
                // Use single line buffer
                renderer.render_by_line(DisplayWrapper {
                    display: &mut *self.display.borrow_mut(),
                    line_buffer: &mut *self.line_buffer.borrow_mut(),
                });
            });

            // Try to put the MCU to sleep
            if !self.window.has_active_animations() {
                if let Some(_duration) = slint::platform::duration_until_next_timer_update() {
                    // TODO: async sleep
                    continue;
                }
            }
        }
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
