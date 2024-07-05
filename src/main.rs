#![no_main]
#![no_std]
#![feature(type_alias_impl_trait)]
#![warn(dead_code)]
extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use defmt::info;
use embedded_alloc::Heap;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use ft6236::FT6236;
use hpm_hal::gpio::{Level, Output, Speed};
use hpm_hal::mode::Blocking;
use hpm_hal::spi::{Config, Spi, Timings, MODE_0};
use hpm_hal::time::Hertz;
use riscv::delay::McycleDelay;
use rm67162::RM67162;
use slint::{LogicalPosition, Model as _};
use {defmt_rtt as _, hpm_hal as hal};

use crate::slint_ui::*;

mod ft6236;
mod rm67162;
mod slint_ui;
struct PrinterQueueData {
    data: Rc<slint::VecModel<PrinterQueueItem>>,
    print_progress_timer: slint::Timer,
}

impl PrinterQueueData {
    fn push_job(&self, title: slint::SharedString) {
        self.data.push(PrinterQueueItem {
            status: JobStatus::Waiting,
            progress: 0,
            title,
            owner: env!("CARGO_PKG_AUTHORS").into(),
            pages: 1,
            size: "100kB".into(),
            submission_date: "".into(),
        })
    }
}

#[global_allocator]
static HEAP: Heap = Heap::empty();

// #[hal::entry]
// fn main() -> ! {
#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let p = hal::init(Default::default());

    // Initialize the allocator BEFORE you use it
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 170 * 1024;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    // Initialize a window (we'll need it later).
    let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
    );

    slint::platform::set_platform(Box::new(MyPlatform { window: window.clone() })).unwrap();

    // Make sure the window covers our entire screen.
    // window.set_size(slint::PhysicalSize::new(600, 450));
    window.set_size(slint::PhysicalSize::new(536, 240));

    let mut delay = McycleDelay::new(hal::sysctl::clocks().cpu0.0);
    defmt::info!("Board init!");

    let mut rst = Output::new(p.PA09, Level::High, Speed::Fast);

    // let mut pwr_en = Output::new(p.PB15, Level::High, Speed::Fast);
    // pwr_en.set_high();

    let mut im = Output::new(p.PB12, Level::High, Speed::Fast);
    im.set_high();

    let mut iovcc = Output::new(p.PB13, Level::High, Speed::Fast);
    iovcc.set_high();

    let spi_config = Config {
        frequency: Hertz(40_000_000),
        mode: MODE_0,
        timing: Timings {
            cs2sclk: hpm_hal::spi::Cs2Sclk::_2HalfSclk,
            csht: hpm_hal::spi::CsHighTime::_4HalfSclk,
        },
        ..Default::default()
    };

    let spi: hal::spi::Spi<'_, Blocking> =
        Spi::new_blocking_quad(p.SPI1, p.PA26, p.PA27, p.PA29, p.PA28, p.PA30, p.PA31, spi_config);

    let mut display = RM67162::new(spi);
    display.reset(&mut rst, &mut delay).unwrap();
    info!("reset display");
    if let Err(e) = display.init(&mut delay) {
        info!("Error: {:?}", e);
        // panic!("Error: {:?}", e);
    }
    info!("clearing display");
    if let Err(e) = display.clear(Rgb565::BLACK) {
        info!("Error: {:?}", e);
        // panic!("Error: {:?}", e);
    }

    // Touch driver
    let i2c_config = hal::i2c::Config::default();
    let i2c = hal::i2c::I2c::new_blocking(p.I2C2, p.PB08, p.PB09, i2c_config);
    let mut touch = FT6236::new_with_addr(i2c, 0x38);
    let mut tp_rst = Output::new(p.PB14, Level::High, Speed::Fast);
    touch.reset(&mut tp_rst, &mut embassy_time::Delay).unwrap();
    touch.init(ft6236::Config::default()).unwrap();

    info!("window set");
    let main_window = MainWindow::new().unwrap();
    main_window.set_ink_levels(
        [
            InkLevel {
                color: slint::Color::from_rgb_u8(0, 255, 255),
                level: 0.40,
            },
            InkLevel {
                color: slint::Color::from_rgb_u8(255, 0, 255),
                level: 0.20,
            },
            InkLevel {
                color: slint::Color::from_rgb_u8(255, 255, 0),
                level: 0.50,
            },
            InkLevel {
                color: slint::Color::from_rgb_u8(0, 0, 0),
                level: 0.80,
            },
        ]
        .into(),
    );

    let default_queue: Vec<PrinterQueueItem> = main_window
        .global::<PrinterQueue>()
        .get_printer_queue()
        .iter()
        .collect();
    let printer_queue = Rc::new(PrinterQueueData {
        data: Rc::new(slint::VecModel::from(default_queue.clone())),
        print_progress_timer: Default::default(),
    });
    main_window
        .global::<PrinterQueue>()
        .set_printer_queue(printer_queue.data.clone().into());

    main_window.on_quit(move || {
        #[cfg(not(target_arch = "wasm32"))]
        slint::quit_event_loop().unwrap();
    });

    let printer_queue_copy = printer_queue.clone();
    main_window.global::<PrinterQueue>().on_start_job(move |title| {
        printer_queue_copy.push_job(title);
    });

    let printer_queue_copy = printer_queue.clone();
    main_window.global::<PrinterQueue>().on_cancel_job(move |idx| {
        printer_queue_copy.data.remove(idx as usize);
    });

    let printer_queue_weak = Rc::downgrade(&printer_queue);
    printer_queue.print_progress_timer.start(
        slint::TimerMode::Repeated,
        core::time::Duration::from_millis(1),
        move || {
            if let Some(printer_queue) = printer_queue_weak.upgrade() {
                if printer_queue.data.row_count() > 0 {
                    let mut top_item = printer_queue.data.row_data(0).unwrap();
                    top_item.progress += 1;
                    top_item.status = JobStatus::Printing;
                    if top_item.progress > 100 {
                        printer_queue.data.remove(0);
                        if printer_queue.data.row_count() == 0 {
                            return;
                        }
                        top_item = printer_queue.data.row_data(0).unwrap();
                    }
                    printer_queue.data.set_row_data(0, top_item);
                } else {
                    printer_queue.data.set_vec(default_queue.clone());
                }
            }
        },
    );

    let mut led = Output::new(p.PA10, Level::Low, Speed::Fast);
    let mut line_buffer = [slint::platform::software_renderer::Rgb565Pixel::default(); 536];
    // let mut line_buffer = [slint::platform::software_renderer::Rgb565Pixel::default(); 600];
    let mut released_cycles = 0;

    info!("Starting event loop");
    loop {
        // Let Slint run the timer hooks and update animations.
        slint::platform::update_timers_and_animations();

        // Check the touch screen or input device using your driver.
        if let Ok(Some(mut point)) = touch.get_point0() {
            // Create event
            released_cycles = 0;
            let x = point.x;
            point.x = point.y;
            point.y = 240 - x;
            info!("Point: {:?}", point);
            let e = match point.event {
                ft6236::EventType::PressDown => slint::platform::WindowEvent::PointerPressed {
                    position: LogicalPosition {
                        x: point.x as f32,
                        y: point.y as f32,
                    },
                    button: slint::platform::PointerEventButton::Left,
                },
                ft6236::EventType::LiftUp => slint::platform::WindowEvent::PointerReleased {
                    position: LogicalPosition {
                        x: point.x as f32,
                        y: point.y as f32,
                    },
                    button: slint::platform::PointerEventButton::Left,
                },
                ft6236::EventType::Contact => slint::platform::WindowEvent::PointerMoved {
                    position: LogicalPosition {
                        x: point.x as f32,
                        y: point.y as f32,
                    },
                },
            };
            window.dispatch_event(e);
        } else {
            released_cycles += 1;
            if released_cycles > 100 {
                window.dispatch_event(slint::platform::WindowEvent::PointerReleased {
                    position: LogicalPosition { x: 0_f32, y: 0_f32 },
                    button: slint::platform::PointerEventButton::Left,
                });

                window.dispatch_event(slint::platform::WindowEvent::PointerExited);
            }
        };

        // Draw the scene if something needs to be drawn.
        window.draw_if_needed(|renderer| {
            // Use single line buffer
            renderer.render_by_line(DisplayWrapper {
                display: &mut display,
                line_buffer: &mut line_buffer,
            });
        });

        // Try to put the MCU to sleep
        if !window.has_active_animations() {
            if let Some(duration) = slint::platform::duration_until_next_timer_update() {
                // embassy_time::Timer::after_millis(duration.as_millis() as u64).await;
                continue;
            }
        }
        led.toggle();
        // embassy_time::Timer::after_millis(1).await;
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("{:?}", defmt::Debug2Format(info));
    loop {}
}
