use iced::widget::canvas::{self, Canvas, Frame, Geometry};
use iced::widget::{button, column, container, row, text};
use iced::{mouse, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::error::Error;
use std::hash::{Hash, Hasher};

#[derive(Debug, Deserialize)]
struct CsvRecord {
    #[serde(rename = "名称")]
    name: String,
    #[serde(rename = "路径")]
    path: String,
    #[serde(rename = "大小")]
    size: Option<u64>,
}

#[derive(Debug, Clone)]
struct FileNode {
    name: String,
    full_path: String,
    is_dir: bool,
    size: u64,
    children: FxHashMap<String, FileNode>,
}

impl FileNode {
    fn new(name: String, full_path: String, is_dir: bool) -> Self {
        Self {
            name,
            full_path,
            is_dir,
            size: 0,
            children: FxHashMap::default(),
        }
    }
}

#[derive(Clone, Debug)]
struct LayoutBlock {
    rect: Rectangle,
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
    color: Color,
}

fn build_tree(file_path: &str) -> Result<FileNode, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(file_path)?;

    let mut root = FileNode::new("Computer".into(), "".into(), true);

    for result in rdr.deserialize() {
        let record: CsvRecord = result?;
        let item_size = record.size.unwrap_or(0);
        let is_dir = record.size.is_none();

        let parts: Vec<&str> = record.path.split('\\').filter(|s| !s.is_empty()).collect();
        let mut current_node = &mut root;
        current_node.size += item_size;

        let mut current_path = String::new();

        for part in parts {
            if current_path.is_empty() {
                current_path.push_str(part);
            } else {
                current_path.push('\\');
                current_path.push_str(part);
            }

            let next_node = current_node
                .children
                .entry(part.to_string())
                .or_insert_with(|| FileNode::new(part.to_string(), current_path.clone(), true));

            next_node.size += item_size;
            current_node = next_node;
        }

        let final_path = if current_path.is_empty() {
            record.name.clone()
        } else {
            format!("{}\\{}", current_path, record.name)
        };

        let target_node = current_node
            .children
            .entry(record.name.clone())
            .or_insert_with(|| FileNode::new(record.name, final_path, is_dir));

        target_node.size += item_size;
        target_node.is_dir = is_dir;
    }

    Ok(root)
}

fn generate_color(name: &str, is_dir: bool) -> Color {
    if !is_dir {
        return Color::from_rgb(0.3, 0.3, 0.35);
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();

    let hue = (hash % 360) as f32;
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

fn compute_treemap(rect: Rectangle, mut nodes: Vec<&FileNode>) -> Vec<LayoutBlock> {
    if nodes.is_empty() {
        return vec![];
    }

    nodes.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    let total_size: u64 = nodes.iter().map(|n| n.size).sum();

    if total_size == 0 {
        return vec![];
    }

    let mut blocks = Vec::new();
    divide_rect(rect, &nodes, total_size, &mut blocks);
    blocks
}

fn divide_rect(rect: Rectangle, nodes: &[&FileNode], total_size: u64, out: &mut Vec<LayoutBlock>) {
    if nodes.is_empty() || total_size == 0 || rect.width <= 1.0 || rect.height <= 1.0 {
        return;
    }

    if nodes.len() == 1 {
        let node = nodes[0];
        out.push(LayoutBlock {
            rect,
            path: node.full_path.clone(),
            name: node.name.clone(),
            size: node.size,
            is_dir: node.is_dir,
            color: generate_color(&node.name, node.is_dir),
        });
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
            Rectangle::new(Point::new(rect.x + w1, rect.y), Size::new(rect.width - w1, rect.height))
        )
    } else {
        let h1 = rect.height * ratio;
        (
            Rectangle::new(rect.position(), Size::new(rect.width, h1)),
            Rectangle::new(Point::new(rect.x, rect.y + h1), Size::new(rect.width, rect.height - h1))
        )
    };

    divide_rect(left_rect, left_nodes, left_size, out);
    divide_rect(right_rect, right_nodes, right_size, out);
}


pub fn main() -> iced::Result {
    iced::application(SpaceSnifferApp::default, SpaceSnifferApp::update, SpaceSnifferApp::view)
        .title("Rusty SpaceSniffer")
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(String),
    GoBack,
}

struct SpaceSnifferApp {
    root_node: FileNode,
    current_path: String,
    path_history: Vec<String>,
}

impl Default for SpaceSnifferApp {
    fn default() -> Self {
        let root = build_tree("test.csv").unwrap_or_else(|e| {
            println!("load test.csv failed: {}", e);
            FileNode::new("Error".into(), "".into(), true)
        });

        Self {
            root_node: root,
            current_path: "".to_string(),
            path_history: Vec::new(),
        }
    }
}

impl SpaceSnifferApp {
    fn find_node<'a>(&'a self, path: &str) -> Option<&'a FileNode> {
        if path.is_empty() {
            return Some(&self.root_node);
        }
        let parts: Vec<&str> = path.split('\\').filter(|s| !s.is_empty()).collect();
        let mut current = &self.root_node;
        for part in parts {
            if let Some(child) = current.children.get(part) {
                current = child;
            } else {
                return None;
            }
        }
        Some(current)
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Navigate(path) => {
                if let Some(node) = self.find_node(&path) {
                    if node.is_dir {
                        self.path_history.push(self.current_path.clone());
                        self.current_path = path;
                    }
                }
            }
            Message::GoBack => {
                if let Some(prev) = self.path_history.pop() {
                    self.current_path = prev;
                }
            }
        }
    }

    fn view(&self) -> Element<Message> {
        let current_node = self.find_node(&self.current_path).unwrap_or(&self.root_node);
        let children: Vec<&FileNode> = current_node.children.values().collect();

        // fix E0599
        let mut top_bar = row![].spacing(10).align_y(iced::Alignment::Center);

        if !self.path_history.is_empty() {
            top_bar = top_bar.push(button("< prev").on_press(Message::GoBack));
        }

        let display_path = if self.current_path.is_empty() {
            "计算机 (根目录)"
        } else {
            &self.current_path
        };

        top_bar = top_bar.push(text(format!("当前位置: {} (TT: {:.2} MB)", display_path, current_node.size as f64 / 1048576.0)).size(18));

        let map = TreemapCanvas { children };
        let canvas = Canvas::new(map).width(Fill).height(Fill);

        container(column![
            container(top_bar).padding(10),
            canvas
        ])
        .width(Fill)
        .height(Fill)
        .into()
    }
}


struct TreemapCanvas<'a> {
    children: Vec<&'a FileNode>,
}

impl<'a> canvas::Program<Message> for TreemapCanvas<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &<Self as canvas::Program<Message>>::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let blocks = compute_treemap(bounds, self.children.clone());

        for block in blocks {
            let draw_rect = Rectangle::new(
                Point::new(block.rect.x + 1.0, block.rect.y + 1.0),
                Size::new((block.rect.width - 2.0).max(0.0), (block.rect.height - 2.0).max(0.0)),
            );

            frame.fill_rectangle(draw_rect.position(), draw_rect.size(), block.color);

            if draw_rect.width > 60.0 && draw_rect.height > 30.0 {
                let size_mb = block.size as f64 / 1048576.0;
                let label = if size_mb > 1024.0 {
                    format!("{}\n{:.2} GB", block.name, size_mb / 1024.0)
                } else {
                    format!("{}\n{:.2} MB", block.name, size_mb)
                };

                let text = canvas::Text {
                    content: label,
                    position: Point::new(draw_rect.x + 5.0, draw_rect.y + 5.0),
                    color: Color::WHITE,
                    size: 14.0.into(),
                    ..Default::default()
                };
                frame.fill_text(text);
            }
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut <Self as canvas::Program<Message>>::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<iced::widget::Action<Message>> {

        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(cursor_pos) = cursor.position_in(bounds) {
                let click_point = Point::new(cursor_pos.x + bounds.x, cursor_pos.y + bounds.y);

                let blocks = compute_treemap(bounds, self.children.clone());
                for block in blocks {
                    if block.rect.contains(click_point) {
                        return Some(iced::widget::Action::publish(Message::Navigate(block.path)));
                    }
                }
            }
        }

        None
    }
}
