use iced::widget::canvas::{self, Frame, Geometry};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::Message;
use crate::data::FileNode;

pub struct LayoutBlock {
    pub rect: Rectangle,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub color: Color,
    pub is_expanded: bool,
}

pub fn format_size(size: u64) -> String {
    let size_f64 = size as f64;
    let kb = 1024.0;
    let mb = kb * 1024.0;
    let gb = mb * 1024.0;
    let tb = gb * 1024.0;

    if size_f64 >= tb {
        format!("{:.2} TB", size_f64 / tb)
    } else if size_f64 >= gb {
        format!("{:.2} GB", size_f64 / gb)
    } else if size_f64 >= mb {
        format!("{:.2} MB", size_f64 / mb)
    } else if size_f64 >= kb {
        format!("{:.2} KB", size_f64 / kb)
    } else {
        format!("{} B", size)
    }
}

pub fn generate_color(name: &str, is_dir: bool) -> Color {
    if !is_dir {
        return Color::from_rgb(0.3, 0.3, 0.35);
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let hue = (hasher.finish() % 360) as f32;
    let saturation: f32 = 0.6;
    let lightness: f32 = 0.5;

    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = lightness - c / 2.0;

    let (r, g, b) = match hue as i32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::from_rgb(r + m, g + m, b + m)
}

fn fit_char_count(max_width: f32, font_size: f32) -> usize {
    ((max_width / (font_size * 0.58)).floor() as usize).max(1)
}

fn ellipsize(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    if max_chars <= 1 {
        return "…".to_string();
    }

    let mut result = text.chars().take(max_chars - 1).collect::<String>();
    result.push('…');
    result
}

fn build_header_label(name: &str, size: u64, max_width: f32, font_size: f32) -> String {
    let full = format!("{} ({})", name, format_size(size));
    ellipsize(&full, fit_char_count(max_width, font_size))
}

fn build_leaf_label(name: &str, size: u64, max_width: f32, font_size: f32) -> String {
    let max_chars = fit_char_count(max_width, font_size);
    let name_line = ellipsize(name, max_chars);
    let size_line = ellipsize(&format_size(size), max_chars);
    format!("{name_line}\n{size_line}")
}

fn draw_overlay_panel(frame: &mut Frame, rect: Rectangle) {
    frame.fill_rectangle(
        Point::new(rect.x + 3.0, rect.y + 4.0),
        rect.size(),
        Color::from_rgba(0.0, 0.0, 0.0, 0.28),
    );

    frame.fill_rectangle(
        rect.position(),
        rect.size(),
        Color::from_rgba(0.06, 0.07, 0.09, 1.0),
    );

    let top = Rectangle::new(rect.position(), Size::new(rect.width, 1.0));
    let left = Rectangle::new(rect.position(), Size::new(1.0, rect.height));
    let right = Rectangle::new(
        Point::new(rect.x + rect.width - 1.0, rect.y),
        Size::new(1.0, rect.height),
    );
    let bottom = Rectangle::new(
        Point::new(rect.x, rect.y + rect.height - 1.0),
        Size::new(rect.width, 1.0),
    );

    for border in [top, left, right, bottom] {
        frame.fill_rectangle(
            border.position(),
            border.size(),
            Color::from_rgba(0.92, 0.95, 1.0, 1.0),
        );
    }
}

fn rectangles_intersect(a: Rectangle, b: Rectangle) -> bool {
    a.x < b.x + b.width
        && a.x + a.width > b.x
        && a.y < b.y + b.height
        && a.y + a.height > b.y
}

fn compute_tooltip_rect(
    block: &LayoutBlock,
    bounds: Rectangle,
    cursor_pos: Point,
) -> Rectangle {
    let path_chars: Vec<char> = block.path.chars().collect();
    let wrapped_path = path_chars
        .chunks(45)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n  ");

    let tooltip_text = format!(
        "名称: {}\n路径: {}\n大小: {}",
        block.name,
        wrapped_path,
        format_size(block.size)
    );
    let line_count = tooltip_text.lines().count() as f32;
    let line_height = 18.0;
    let padding = 12.0;

    let tooltip_w = 340.0_f32;
    let tooltip_h = padding * 2.0 + line_count * line_height;

    let mut tooltip_x = cursor_pos.x + 15.0;
    let mut tooltip_y = cursor_pos.y + 15.0;

    if tooltip_x + tooltip_w > bounds.width {
        tooltip_x = cursor_pos.x - tooltip_w - 15.0;
    }
    if tooltip_y + tooltip_h > bounds.height {
        tooltip_y = cursor_pos.y - tooltip_h - 15.0;
    }
    tooltip_x = tooltip_x.max(5.0);
    tooltip_y = tooltip_y.max(5.0);

    Rectangle::new(
        Point::new(tooltip_x, tooltip_y),
        Size::new(tooltip_w, tooltip_h),
    )
}

fn label_color(blocked_by_overlay: bool, default: Color) -> Color {
    if blocked_by_overlay {
        Color::from_rgba(0.0, 0.0, 0.0, 0.5)
    } else {
        default
    }
}

pub fn compute_treemap(
    rect: Rectangle,
    root_node: &FileNode,
    expanded_paths: &FxHashSet<String>,
) -> Vec<LayoutBlock> {
    let mut blocks = Vec::new();
    layout_node(root_node, rect, expanded_paths, &mut blocks, true);
    blocks
}

fn layout_node(
    node: &FileNode,
    rect: Rectangle,
    expanded_paths: &FxHashSet<String>,
    out: &mut Vec<LayoutBlock>,
    is_root: bool,
) {
    if rect.width <= 1.0 || rect.height <= 1.0 {
        return;
    }

    let should_expand = is_root || expanded_paths.contains(&node.full_path);

    if !should_expand || node.children.is_empty() {
        out.push(LayoutBlock {
            rect,
            path: node.full_path.clone(),
            name: node.name.clone(),
            size: node.size,
            is_dir: node.is_dir,
            color: generate_color(&node.name, node.is_dir),
            is_expanded: false,
        });
    } else {
        out.push(LayoutBlock {
            rect,
            path: node.full_path.clone(),
            name: node.name.clone(),
            size: node.size,
            is_dir: true,
            color: generate_color(&node.name, true),
            is_expanded: true,
        });

        let header_height = 24.0;
        let padding = 4.0;
        if rect.width > padding * 2.0 + 10.0 && rect.height > header_height + padding + 10.0 {
            let child_rect = Rectangle::new(
                Point::new(rect.x + padding, rect.y + header_height),
                Size::new(
                    rect.width - padding * 2.0,
                    rect.height - header_height - padding,
                ),
            );

            let mut children: Vec<&FileNode> = node.children.values().collect();
            children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
            let total_size: u64 = children.iter().map(|n| n.size).sum();

            divide_and_layout(child_rect, &children, total_size, expanded_paths, out);
        }
    }
}

fn divide_and_layout(
    rect: Rectangle,
    nodes: &[&FileNode],
    total_size: u64,
    expanded_paths: &FxHashSet<String>,
    out: &mut Vec<LayoutBlock>,
) {
    if nodes.is_empty() || total_size == 0 || rect.width <= 1.0 || rect.height <= 1.0 {
        return;
    }

    if nodes.len() == 1 {
        layout_node(nodes[0], rect, expanded_paths, out, false);
        return;
    }

    let mut sum = 0;
    let mut split_idx = 0;
    for (i, node) in nodes.iter().enumerate() {
        sum += node.size;
        split_idx = i;
        if sum >= total_size / 2 {
            break;
        }
    }

    if split_idx == nodes.len() - 1 && nodes.len() > 1 {
        split_idx -= 1;
    }
    let split_idx = split_idx + 1;

    let left_nodes = &nodes[..split_idx];
    let right_nodes = &nodes[split_idx..];

    let left_size: u64 = left_nodes.iter().map(|n| n.size).sum();
    let right_size = total_size - left_size;
    let ratio = left_size as f32 / total_size as f32;

    let (left_rect, right_rect) = if rect.width > rect.height {
        let w1 = rect.width * ratio;
        (
            Rectangle::new(rect.position(), Size::new(w1, rect.height)),
            Rectangle::new(
                Point::new(rect.x + w1, rect.y),
                Size::new(rect.width - w1, rect.height),
            ),
        )
    } else {
        let h1 = rect.height * ratio;
        (
            Rectangle::new(rect.position(), Size::new(rect.width, h1)),
            Rectangle::new(
                Point::new(rect.x, rect.y + h1),
                Size::new(rect.width, rect.height - h1),
            ),
        )
    };

    divide_and_layout(left_rect, left_nodes, left_size, expanded_paths, out);
    divide_and_layout(right_rect, right_nodes, right_size, expanded_paths, out);
}

fn get_parent_path(path: &str) -> &str {
    path.rfind('\\').map(|idx| &path[..idx]).unwrap_or("")
}

#[derive(Default)]
pub struct CanvasState {
    visual_rects: RefCell<FxHashMap<String, Rectangle>>,
    hover_alphas: RefCell<FxHashMap<String, f32>>,
    current_hover: Option<String>,
    last_click: Option<(String, Instant)>,
    pub context_menu: Option<(Point, String)>,
}

pub struct TreemapCanvas<'a> {
    pub root_node: &'a FileNode,
    pub expanded_paths: &'a FxHashSet<String>,
}

impl<'a> canvas::Program<Message> for TreemapCanvas<'a> {
    type State = CanvasState;

    fn draw(
        &self,
        state: &<Self as canvas::Program<Message>>::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut base_frame = Frame::new(renderer, bounds.size());
        let mut overlay_frame = Frame::new(renderer, bounds.size());

        let local_bounds = Rectangle::new(Point::ORIGIN, bounds.size());
        let ideal_blocks = compute_treemap(local_bounds, self.root_node, self.expanded_paths);

        const RECT_SPEED: f32 = 0.08;
        const HOVER_SPEED: f32 = 0.2;

        // physics
        {
            let mut vr = state.visual_rects.borrow_mut();
            let mut ha = state.hover_alphas.borrow_mut();
            let ideal_paths: FxHashSet<_> = ideal_blocks.iter().map(|b| b.path.clone()).collect();

            let root_current = vr
                .entry(self.root_node.full_path.clone())
                .or_insert(local_bounds);
            root_current.x += (local_bounds.x - root_current.x) * RECT_SPEED;
            root_current.y += (local_bounds.y - root_current.y) * RECT_SPEED;
            root_current.width += (local_bounds.width - root_current.width) * RECT_SPEED;
            root_current.height += (local_bounds.height - root_current.height) * RECT_SPEED;

            for block in ideal_blocks.iter() {
                let target = block.rect;
                let current = if let Some(rect) = vr.get_mut(&block.path) {
                    rect
                } else {
                    let parent_path = get_parent_path(&block.path);
                    let start_rect = vr.get(parent_path).copied().unwrap_or(target);
                    vr.entry(block.path.clone()).or_insert(start_rect)
                };

                current.x += (target.x - current.x) * RECT_SPEED;
                current.y += (target.y - current.y) * RECT_SPEED;
                current.width += (target.width - current.width) * RECT_SPEED;
                current.height += (target.height - current.height) * RECT_SPEED;

                let target_alpha = if state.current_hover.as_ref() == Some(&block.path) {
                    1.0
                } else {
                    0.0
                };
                let current_alpha = ha.entry(block.path.clone()).or_insert(0.0);
                *current_alpha += (target_alpha - *current_alpha) * HOVER_SPEED;
            }

            vr.retain(|path, _| {
                ideal_paths.contains(path)
                    || path == &self.root_node.full_path
                    || self.root_node.full_path.starts_with(path)
            });
            ha.retain(|path, _| ideal_paths.contains(path));
        }

        // bg rect
        let vr = state.visual_rects.borrow();
        let ha = state.hover_alphas.borrow();

        let tooltip_overlay_rect = if let Some(hover_path) = &state.current_hover {
            if state.context_menu.is_none() {
                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    ideal_blocks
                        .iter()
                        .find(|b| &b.path == hover_path)
                        .map(|block| compute_tooltip_rect(block, bounds, cursor_pos))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let menu_overlay_rect = state.context_menu.as_ref().map(|(menu_pos, _)| {
            Rectangle::new(*menu_pos, Size::new(160.0, 35.0 * 3.0))
        });

        let blocking_overlay_rect = menu_overlay_rect.or(tooltip_overlay_rect);

        for block in &ideal_blocks {
            let rect = vr.get(&block.path).copied().unwrap_or(block.rect);
            let draw_rect = Rectangle::new(
                Point::new(rect.x + 1.0, rect.y + 1.0),
                Size::new((rect.width - 2.0).max(0.0), (rect.height - 2.0).max(0.0)),
            );

            if block.is_expanded {
                let bg_color = Color::from_rgb(
                    block.color.r * 0.4,
                    block.color.g * 0.4,
                    block.color.b * 0.4,
                );
                base_frame.fill_rectangle(draw_rect.position(), draw_rect.size(), bg_color);

                if draw_rect.height > 20.0 {
                    let header_region = Rectangle::new(
                        Point::new(draw_rect.x + 2.0, draw_rect.y + 2.0),
                        Size::new((draw_rect.width - 4.0).max(0.0), 18.0),
                    );
                    let is_blocked = blocking_overlay_rect
                        .is_some_and(|overlay| rectangles_intersect(header_region, overlay));

                    let header_label = build_header_label(
                        &block.name,
                        block.size,
                        header_region.width - 8.0,
                        14.0,
                    );

                    base_frame.with_clip(header_region, |frame| {
                        frame.fill_text(canvas::Text {
                            content: header_label,
                            position: Point::new(header_region.x + 4.0, header_region.y + 1.0),
                            max_width: (header_region.width - 8.0).max(1.0),
                            color: label_color(is_blocked, Color::from_rgba(1.0, 1.0, 1.0, 0.8)),
                            size: 14.0.into(),
                            ..Default::default()
                        });
                    });
                }
            } else {
                base_frame.fill_rectangle(draw_rect.position(), draw_rect.size(), block.color);

                let hover_alpha = ha.get(&block.path).copied().unwrap_or(0.0);
                if hover_alpha > 0.01 {
                    base_frame.fill_rectangle(
                        draw_rect.position(),
                        draw_rect.size(),
                        Color::from_rgba(1.0, 1.0, 1.0, 0.25 * hover_alpha),
                    );
                }

                if draw_rect.width > 60.0 && draw_rect.height > 30.0 {
                    let text_region = Rectangle::new(
                        Point::new(draw_rect.x + 3.0, draw_rect.y + 3.0),
                        Size::new((draw_rect.width - 6.0).max(0.0), (draw_rect.height - 6.0).max(0.0)),
                    );
                    let is_blocked = blocking_overlay_rect
                        .is_some_and(|overlay| rectangles_intersect(text_region, overlay));

                    let label = build_leaf_label(
                        &block.name,
                        block.size,
                        text_region.width - 4.0,
                        14.0,
                    );

                    base_frame.with_clip(text_region, |frame| {
                        frame.fill_text(canvas::Text {
                            content: label,
                            position: Point::new(text_region.x + 2.0, text_region.y + 2.0),
                            max_width: (text_region.width - 4.0).max(1.0),
                            color: label_color(is_blocked, Color::WHITE),
                            size: 14.0.into(),
                            ..Default::default()
                        });
                    });
                }
            }
        }

        // overlays
        if let Some(hover_path) = &state.current_hover && state.context_menu.is_none() {
            if let Some(cursor_pos) = cursor.position_in(bounds) {
                if let Some(block) = ideal_blocks.iter().find(|b| &b.path == hover_path) {
                    let path_chars: Vec<char> = block.path.chars().collect();
                    let wrapped_path = path_chars
                        .chunks(45)
                        .map(|c| c.iter().collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n  ");

                    let tooltip_text = format!(
                        "名称: {}\n路径: {}\n大小: {}",
                        block.name,
                        wrapped_path,
                        format_size(block.size)
                    );
                    let padding = 12.0;
                    let tooltip_rect = compute_tooltip_rect(block, bounds, cursor_pos);

                    draw_overlay_panel(&mut overlay_frame, tooltip_rect);

                    overlay_frame.fill_text(canvas::Text {
                        content: tooltip_text,
                        position: Point::new(
                            tooltip_rect.x + padding,
                            tooltip_rect.y + padding - 2.0,
                        ),
                        max_width: (tooltip_rect.width - padding * 2.0).max(1.0),
                        color: Color::WHITE,
                        size: 13.0.into(),
                        ..Default::default()
                    });
                }
            }
        }

        if let Some((menu_pos, _path)) = &state.context_menu {
            let menu_w = 160.0;
            let item_h = 35.0;
            let menu_h = item_h * 3.0;

            let menu_rect = Rectangle::new(*menu_pos, Size::new(menu_w, menu_h));

            draw_overlay_panel(&mut overlay_frame, menu_rect);

            let items = ["在资源管理器中显示", "重命名", "移至回收站"];
            for (i, label) in items.iter().enumerate() {
                let item_y = menu_pos.y + (i as f32) * item_h;

                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    if cursor_pos.x >= menu_pos.x
                        && cursor_pos.x <= menu_pos.x + menu_w
                        && cursor_pos.y >= item_y
                        && cursor_pos.y <= item_y + item_h
                    {
                        overlay_frame.fill_rectangle(
                            Point::new(menu_pos.x, item_y),
                            Size::new(menu_w, item_h),
                            Color::from_rgba(1.0, 1.0, 1.0, 0.1),
                        );
                    }
                }

                overlay_frame.fill_text(canvas::Text {
                    content: label.to_string(),
                    position: Point::new(menu_pos.x + 15.0, item_y + 8.0),
                    color: if i == 2 {
                        Color::from_rgb(1.0, 0.4, 0.4)
                    } else {
                        Color::WHITE
                    },
                    size: 14.0.into(),
                    ..Default::default()
                });
            }
        }

        vec![base_frame.into_geometry(), overlay_frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut <Self as canvas::Program<Message>>::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<iced::widget::Action<Message>> {
        let local_bounds = Rectangle::new(Point::ORIGIN, bounds.size());
        let ideal_blocks = compute_treemap(local_bounds, self.root_node, self.expanded_paths);

        if let iced::Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            if let Some(local_cursor) = cursor.position_in(bounds) {
                state.current_hover = None;
                for block in ideal_blocks.iter().rev() {
                    if block.rect.contains(local_cursor) {
                        state.current_hover = Some(block.path.clone());
                        break;
                    }
                }
            } else {
                state.current_hover = None;
            }
        }

        if let iced::Event::Mouse(mouse::Event::ButtonPressed(button)) = event {
            if let Some(local_cursor) = cursor.position_in(bounds) {
                if button.clone() == mouse::Button::Left && state.context_menu.is_some() {
                    let (menu_pos, target_path) = state.context_menu.clone().unwrap();
                    let menu_w = 160.0;
                    let item_h = 35.0;

                    if local_cursor.x >= menu_pos.x
                        && local_cursor.x <= menu_pos.x + menu_w
                        && local_cursor.y >= menu_pos.y
                        && local_cursor.y <= menu_pos.y + item_h * 3.0
                    {
                        let click_index = ((local_cursor.y - menu_pos.y) / item_h) as usize;
                        state.context_menu = None;

                        return match click_index {
                            0 => Some(iced::widget::Action::publish(Message::OpenInExplorer(
                                target_path,
                            ))),
                            1 => Some(iced::widget::Action::publish(Message::RequestRename(
                                target_path,
                            ))),
                            2 => Some(iced::widget::Action::publish(Message::RequestDelete(
                                target_path,
                            ))),
                            _ => None,
                        };
                    } else {
                        state.context_menu = None;
                        return None;
                    }
                }

                for block in ideal_blocks.iter().rev() {
                    if block.rect.contains(local_cursor) {
                        if button.clone() == mouse::Button::Right {
                            let mut menu_x = local_cursor.x;
                            let mut menu_y = local_cursor.y;
                            if menu_x + 160.0 > bounds.width {
                                menu_x = bounds.width - 160.0;
                            }
                            if menu_y + 105.0 > bounds.height {
                                menu_y = bounds.height - 105.0;
                            }

                            state.context_menu =
                                Some((Point::new(menu_x, menu_y), block.path.clone()));
                            return None;
                        }

                        if button.clone() == mouse::Button::Left {
                            let now = Instant::now();

                            if let Some((last_path, time)) = &state.last_click {
                                if block.path.starts_with(last_path)
                                    && now.duration_since(*time).as_millis() < 300
                                {
                                    if block.is_dir {
                                        return Some(iced::widget::Action::publish(
                                            Message::Navigate(last_path.clone()),
                                        ));
                                    }
                                }
                            }

                            state.last_click = Some((block.path.clone(), now));

                            if block.is_dir && !block.is_expanded {
                                return Some(iced::widget::Action::publish(Message::ToggleExpand(
                                    block.path.clone(),
                                )));
                            }
                        }
                        break;
                    }
                }
            }
        }
        None
    }
}
