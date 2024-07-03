#![no_main]
#![no_std]
#![feature(type_alias_impl_trait)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use embassy_executor::Spawner;
use core::cell::RefCell;

use ::slint::platform::software_renderer::MinimalSoftwareWindow;
use defmt::info;
use embedded_alloc::Heap;
use embedded_graphics::draw_target::DrawTarget;
// use embedded_graphics::geometry::{Dimensions, Point, Size};
// use embedded_graphics::image::{Image, ImageRawLE};
// use embedded_graphics::mono_font::ascii::FONT_10X20;
// use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_hal::delay::DelayNs;
// use embedded_graphics::primitives::Rectangle;
// use embedded_graphics::text::Text;
// use embedded_graphics::transform::Transform;
// use embedded_graphics::Drawable;
use hpm_hal::gpio::{Level, Output, Speed};
use hpm_hal::mode::Blocking;
use hpm_hal::spi::{Config, Spi, Timings, MODE_0};
use hpm_hal::time::Hertz;
use riscv::delay::McycleDelay;
use rm67162::RM67162;
use slint::Model as _;
use {defmt_rtt as _, hpm_hal as hal};

use crate::slint_ui::*;

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

const HEAP_SIZE: usize = 10 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
#[global_allocator]
static ALLOCATOR: Heap = Heap::empty();
// #[hal::entry]
// fn main() -> ! {
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = hal::init(Default::default());

    // Initialize heap
    unsafe { ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP) as usize, HEAP_SIZE) }

    // let mut delay = McycleDelay::new(hal::sysctl::clocks().cpu0.0);
    defmt::info!("Board init!");

    let mut rst = Output::new(p.PA09, Level::High, Speed::Fast);

    let mut im = Output::new(p.PB12, Level::High, Speed::Fast);
    im.set_high();

    let mut iovcc = Output::new(p.PB13, Level::High, Speed::Fast);
    iovcc.set_high();

    // PA10
    let mut led = Output::new(p.PA10, Level::Low, Speed::Fast);

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
    let mut delay = embassy_time::Delay;
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

    let window = MinimalSoftwareWindow::new(Default::default());
    slint::platform::set_platform(Box::new(slint_ui::MyPlatform {
        window: window.clone(),
        display: RefCell::new(display),
        line_buffer: RefCell::new([slint::platform::software_renderer::Rgb565Pixel(0); 536]),
    }))
    .unwrap();

    // Setup the UI.
    // let _ui = MainWindow::new();
    // ... setup callback and properties on `ui` ...

    // Make sure the window covers our entire screen.
    window.set_size(slint::PhysicalSize::new(536, 240));

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

    info!("main window");
    main_window.run().unwrap();

    loop {
        led.toggle();
        embassy_time::Timer::after_secs(1).await
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("{:?}", defmt::Debug2Format(info));
    loop {}
}
