use iced::{Background, Border, Color, Gradient, Radians, gradient::Linear, widget::{button, container}};


pub fn button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::WHITE)),
        text_color: Color::BLACK,
        border: Border {
            radius: 12.0.into(),
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

pub fn idle_container_style(_theme: &iced::Theme) -> container::Style {
    let linear = Linear::new(Radians(std::f32::consts::FRAC_PI_4))
        .add_stop(0.0, Color::from_rgb8(15, 17, 20))
        .add_stop(1.0, Color::from_rgb8(30, 34, 40));

    container::Style {
        background: Some(Background::Gradient(Gradient::Linear(linear))),
        ..container::Style::default()
    }
}

pub fn recommend_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::from_rgb8(38, 166, 154))),
        text_color: Color::WHITE,
        border: Border {
            radius: 12.0.into(),
            width: 1.0,
            color: Color::from_rgb8(77, 208, 196),
        },
        shadow: iced::Shadow {
            color: Color::from_rgba8(77, 208, 196, 0.1),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 8.0,
        },
        ..button::Style::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb8(77, 208, 196))),
            border: Border {
                color: Color::WHITE,
                ..base.border
            },
            shadow: iced::Shadow {
                color: Color::from_rgba8(77, 208, 196, 0.25),
                blur_radius: 12.0,
                ..base.shadow
            },
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb8(0, 137, 123))),
            shadow: iced::Shadow::default(),
            ..base
        },
        _ => base,
    }
}

pub fn secondary_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::from_rgba8(255, 255, 255, 0.05))),
        text_color: Color::from_rgb8(220, 220, 220),
        border: Border {
            radius: 12.0.into(),
            width: 1.0,
            color: Color::from_rgba8(255, 255, 255, 0.1),
        },
        ..button::Style::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgba8(255, 255, 255, 0.12))),
            text_color: Color::WHITE,
            border: Border {
                color: Color::from_rgba8(255, 255, 255, 0.25),
                ..base.border
            },
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgba8(255, 255, 255, 0.03))),
            ..base
        },
        _ => base,
    }
}
