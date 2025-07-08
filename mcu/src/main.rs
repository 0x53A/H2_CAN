#![no_std]
#![no_main]
#![feature(never_type)]

use core::fmt::Write;

use embedded_can::{self, *};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    i2c::{self, master::I2c},
    main,
    peripherals::{GPIO17, GPIO18, GPIO21, I2C0},
    time::Rate,
    twai::{
        self,
        filter::SingleStandardFilter,
        *,
    },
};

use embedded_graphics::mono_font::ascii::FONT_10X20;

use anyhow::Result;
use heapless::String;
use ssd1306::mode::DisplayConfig;

esp_bootloader_esp_idf::esp_app_desc!();

use esp_alloc as _;

// --

type MyDisplay = ssd1306::Ssd1306<
    ssd1306::prelude::I2CInterface<I2c<'static, esp_hal::Blocking>>,
    ssd1306::prelude::DisplaySize128x64,
    ssd1306::mode::BufferedGraphicsMode<ssd1306::prelude::DisplaySize128x64>,
>;

#[main]
fn main() -> ! {
    esp_alloc::heap_allocator!(size: 32 * 1024);

    match _main() {
        Ok(_) => loop {},
        Err(_e) => loop {},
    }
}

fn _main() -> Result<!> {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = esp_hal::delay::Delay::new();

    let mut str: String<100> = heapless::String::new();

    let mut display = init_display(
        peripherals.GPIO21,
        peripherals.I2C0,
        peripherals.GPIO17,
        peripherals.GPIO18,
        &delay,
    )?;

    let twai_rx_pin = peripherals.GPIO47;
    let twai_tx_pin = peripherals.GPIO48;

    const TWAI_BAUDRATE: twai::BaudRate = BaudRate::B500K;

    let mut twai_config = twai::TwaiConfiguration::new(
        peripherals.TWAI0,
        twai_rx_pin,
        twai_tx_pin,
        TWAI_BAUDRATE,
        TwaiMode::Normal,
    );

    // todo: test what happens if no filter is set - are all frames received or none?
    twai_config.set_filter(
        const { SingleStandardFilter::new(b"xxxxxxxxxxx", b"x", [b"xxxxxxxx", b"xxxxxxxx"]) },
    );

    let mut twai = twai_config.start();

    write!(&mut str, "Started TWAI")?;
    set_status(&mut display, &str)?;

    let mut last_percentage = -1.0;

    loop {
        match twai.receive() {
            Ok(frame) => {
                let data = frame.data();

                let frame_id = frame.id();
                let is_h2_data = match frame_id {
                    embedded_can::Id::Standard(std_id) => {
                        let id_val = std_id.as_raw();
                        matches!(
                            id_val,
                            0x300 | 0x308 | 0x310 | 0x318 |  // NEO974A
                                0x320 | 0x328 | 0x330 | 0x338 |  // NEO983A  
                                0x340 | 0x348 | 0x350 | 0x358 // NEO986A
                        )
                    }
                    _ => false,
                };

                if is_h2_data && data.len() >= 2 {
                    let msg0 = u16::from_be_bytes([data[0], data[1]]);

                    let h2_percentage = (msg0 as f32 - 20.0) / 100.0;

                    if last_percentage != h2_percentage {
                        last_percentage = h2_percentage;

                        str.clear();
                        write!(&mut str, "H2: {:.4}%", h2_percentage)?;
                        set_status(&mut display, &str)?;

                        esp_println::println!("H2: {:.4}%", h2_percentage);
                    }
                }
            }
            Err(::nb::Error::WouldBlock) => {}
            Err(e) => {
                esp_println::println!("Error receiving TWAI frame: {:?}", e);
            }
        }
    }
}

fn init_display(
    rst: GPIO21<'static>,
    i2c: I2C0<'static>,
    sda: GPIO17<'static>,
    scl: GPIO18<'static>,
    delay: &Delay,
) -> Result<MyDisplay> {
    esp_println::dbg!("About to initialize the Heltec SSD1306 I2C LED driver");

    let i2c_config = i2c::master::Config::default().with_frequency(Rate::from_khz(400));

    let i2c = I2c::new(i2c, i2c_config)?.with_scl(scl).with_sda(sda);

    let di = ssd1306::I2CDisplayInterface::new(i2c);

    let mut reset = Output::new(rst, Level::High, OutputConfig::default());

    // high for 1 ms
    delay.delay_millis(1 as u32);

    reset.set_low();
    delay.delay_millis(10 as u32);

    reset.set_high();

    // Note:
    //   PinDriver of IDF had a Drop implementation that resets the pin, which would turn off the display
    //   This seems to no longer be neccessary when using esp-hal.
    // mem::forget(reset);

    let mut display: ssd1306::Ssd1306<
        ssd1306::prelude::I2CInterface<I2c<'_, esp_hal::Blocking>>,
        ssd1306::prelude::DisplaySize128x64,
        ssd1306::mode::BufferedGraphicsMode<ssd1306::prelude::DisplaySize128x64>,
    > = ssd1306::Ssd1306::new(
        di,
        ssd1306::size::DisplaySize128x64,
        ssd1306::rotation::DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();

    display
        .init()
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    write_text(
        &mut display,
        "Hello Rust!",
        BinaryColor::Off,
        BinaryColor::On,
        BinaryColor::Off,
        BinaryColor::On,
    )
    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    display
        .flush()
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    Ok(display)
}

fn write_text<D>(
    display: &mut D,
    text: &str,
    bg: D::Color,
    fg: D::Color,
    fill: D::Color,
    stroke: D::Color,
) -> Result<(), D::Error>
where
    D: DrawTarget + Dimensions,
{
    display.clear(bg)?;

    Rectangle::new(display.bounding_box().top_left, display.bounding_box().size)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(fill)
                .stroke_color(stroke)
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;

    Text::new(
        &text,
        Point::new(10, (display.bounding_box().size.height - 10) as i32 / 2),
        MonoTextStyle::new(&FONT_10X20, fg),
    )
    .draw(display)?;

    Ok(())
}

fn set_status(display: &mut MyDisplay, text: &str) -> Result<()> {
    write_text(
        display,
        &text,
        BinaryColor::Off,
        BinaryColor::On,
        BinaryColor::Off,
        BinaryColor::On,
    )
    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    display
        .flush()
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    return Ok(());
}
