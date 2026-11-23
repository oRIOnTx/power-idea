//! ANSI palette mapped onto the active IDE theme.

use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

pub(super) fn to_rgb(color: Hsla) -> Rgb {
    let rgba = color.to_rgb();
    Rgb {
        r: (rgba.r * 255.) as u8,
        g: (rgba.g * 255.) as u8,
        b: (rgba.b * 255.) as u8,
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}

/// Maps ANSI colors onto the active theme so the terminal matches the IDE.
pub(super) fn ansi_color(color: AnsiColor, cx: &App) -> Hsla {
    let theme: &Theme = cx.theme();
    match color {
        AnsiColor::Spec(c) => rgb(c.r, c.g, c.b),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
            NamedColor::DimForeground => theme.muted_foreground,
            NamedColor::Background => theme.background,
            NamedColor::Cursor => theme.caret,
            other => indexed_color(other as usize, theme),
        },
        AnsiColor::Indexed(ix) => indexed_color(ix as usize, theme),
    }
}

fn indexed_color(ix: usize, theme: &Theme) -> Hsla {
    let dark = theme.mode.is_dark();
    match ix {
        0 => {
            if dark {
                rgb(0x3b, 0x3f, 0x45)
            } else {
                rgb(0x24, 0x29, 0x2e)
            }
        }
        1 => theme.red,
        2 => theme.green,
        3 => theme.yellow,
        4 => theme.blue,
        5 => theme.magenta,
        6 => theme.cyan,
        7 => {
            if dark {
                rgb(0xc9, 0xcd, 0xd3)
            } else {
                rgb(0x6a, 0x73, 0x7d)
            }
        }
        8 => theme.muted_foreground,
        9 => theme.red_light,
        10 => theme.green_light,
        11 => theme.yellow_light,
        12 => theme.blue_light,
        13 => theme.magenta_light,
        14 => theme.cyan_light,
        15 => {
            if dark {
                rgb(0xff, 0xff, 0xff)
            } else {
                rgb(0x2b, 0x30, 0x36)
            }
        }
        16..=231 => {
            let ix = ix - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            rgb(level(ix / 36), level((ix / 6) % 6), level(ix % 6))
        }
        232..=255 => {
            let v = (8 + (ix - 232) * 10) as u8;
            rgb(v, v, v)
        }
        _ => theme.foreground,
    }
}
