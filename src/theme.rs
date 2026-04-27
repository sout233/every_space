use iced::{Background, Border, Color, Gradient, Radians, gradient::Linear, widget::{button, container}};


pub fn button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::WHITE)),
        text_color: Color::BLACK,
        border: Border {
            radius: 64.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        ..button::Style::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.8, 0.8, 0.8))),
            ..base
        },
        _ => base,
    }
}

pub fn succeed_container_style(_theme: &iced::Theme) -> container::Style {
    let linear = Linear::new(Radians(std::f32::consts::FRAC_PI_2))
        .add_stop(0.0, Color::from_rgb8(30, 66, 64))
        .add_stop(1.0, Color::from_rgb8(33, 37, 43));

    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(linear))),
        ..container::Style::default()
    }
}
