mod circular;
mod data;
mod easing;
mod treemap;
mod theme;

use iced::time::Instant;
use iced::widget::{Canvas, button, column, container, row, text, text_input};
use iced::{Element, Fill, Padding, Task, window};
use rustc_hash::FxHashSet;
use std::env;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, channel};
use tokio::runtime::Runtime;

use circular::Circular;
use data::{DataSourceKind, FileNode};
use treemap::TreemapCanvas;

use crate::theme::{button_style, succeed_container_style};

fn get_tokio_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("init tokio runtime failed")
    })
}

pub fn main() -> iced::Result {
    iced::application(
        SpaceSnifferApp::default,
        SpaceSnifferApp::update,
        SpaceSnifferApp::view,
    )
    .title("EverySpace")
    .subscription(SpaceSnifferApp::subscription)
    .run()
}

#[derive(Debug, Clone)]
pub enum Message {
    AutoLoadSystemMft,
    PickCsvFile,
    FilePicked(DataSourceKind, Option<String>),
    Tick(Instant),
    ToggleExpand(String),
    Navigate(String),
    GoBack,
    OpenInExplorer(String),
    RequestDelete(String),
    FileDeleted(String, bool),
    RequestRename(String),
    RenameInputChanged(String),
    ConfirmRename,
    CancelRename,
}

enum AppState {
    Idle,
    Loading {
        source_kind: DataSourceKind,
        source_path: String,
        root_node: FileNode,
        current_path: String,
        expanded_paths: FxHashSet<String>,
        rx: Receiver<Result<FileNode, String>>,
        anim_tick: usize,
    },
    Loaded {
        source_kind: DataSourceKind,
        source_path: String,
        root_node: FileNode,
        current_path: String,
        path_history: Vec<String>,
        expanded_paths: FxHashSet<String>,
        anim_tick: usize,
        rename_target: Option<(String, String)>, // 原路径
    },
    Error(String),
}

impl Default for AppState {
    fn default() -> Self {
        AppState::Idle
    }
}

struct SpaceSnifferApp {
    state: AppState,
    auto_load_started: bool,
}

impl Default for SpaceSnifferApp {
    fn default() -> Self {
        Self {
            state: AppState::Idle,
            auto_load_started: false,
        }
    }
}

impl SpaceSnifferApp {
    fn resolve_system_mft_path() -> Result<String, String> {
        let system_drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        Ok(format!("{system_drive}\\$MFT"))
    }

    fn start_loading(&mut self, source_kind: DataSourceKind, path: String) -> Task<Message> {
        let (tx, rx) = channel::<Result<FileNode, String>>();

        data::build_tree_stream(source_kind, path.clone(), tx);

        self.state = AppState::Loading {
            source_kind,
            source_path: path,
            root_node: FileNode::root(),
            current_path: "".to_string(),
            expanded_paths: FxHashSet::default(),
            rx,
            anim_tick: 0,
        };

        Task::none()
    }

    fn find_node<'a>(root: &'a FileNode, path: &str) -> Option<&'a FileNode> {
        if path.is_empty() {
            return Some(root);
        }
        let parts: Vec<&str> = path.split('\\').filter(|s| !s.is_empty()).collect();
        let mut current = root;
        for part in parts {
            if let Some(child) = current.children.get(part) {
                current = child;
            } else {
                return None;
            }
        }
        Some(current)
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        window::frames().map(Message::Tick)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AutoLoadSystemMft => {
                self.auto_load_started = true;
                match Self::resolve_system_mft_path() {
                    Ok(path) => self.start_loading(DataSourceKind::Mft, path),
                    Err(err) => {
                        self.state = AppState::Error(err);
                        Task::none()
                    }
                }
            }
            Message::PickCsvFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("CSV Data", &["csv"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_string_lossy().to_string())
                },
                |path| Message::FilePicked(DataSourceKind::Csv, path),
            ),
            Message::FilePicked(source_kind, Some(path)) => self.start_loading(source_kind, path),
            Message::FilePicked(_, None) => Task::none(),
            Message::Tick(_now) => {
                if matches!(self.state, AppState::Idle) && !self.auto_load_started {
                    return Task::done(Message::AutoLoadSystemMft);
                }

                let mut is_finished_loading = false;
                let mut error_msg = None;

                if let AppState::Loading {
                    root_node,
                    rx,
                    anim_tick,
                    expanded_paths: _,
                    ..
                } = &mut self.state
                {
                    *anim_tick = anim_tick.wrapping_add(1);

                    loop {
                        match rx.try_recv() {
                            Ok(Ok(partial_tree)) => {
                                data::merge_into(root_node, partial_tree);
                            }
                            Ok(Err(e)) => {
                                error_msg = Some(e);
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                is_finished_loading = true;
                                break;
                            }
                        }
                    }
                } else if let AppState::Loaded { anim_tick, .. } = &mut self.state {
                    *anim_tick = anim_tick.wrapping_add(1);
                }

                if let Some(err) = error_msg {
                    self.state = AppState::Error(err);
                    return Task::none();
                }

                if is_finished_loading {
                    let mut temp_state = AppState::Idle;
                    std::mem::swap(&mut self.state, &mut temp_state);

                    if let AppState::Loading {
                        source_kind,
                        source_path,
                        root_node,
                        expanded_paths,
                        ..
                    } = temp_state
                    {
                        self.state = AppState::Loaded {
                            source_kind,
                            source_path,
                            root_node,
                            current_path: "".to_string(),
                            path_history: Vec::new(),
                            expanded_paths,
                            anim_tick: 0,
                            rename_target: None,
                        };
                    }
                }
                Task::none()
            }
            Message::ToggleExpand(path) => {
                if let AppState::Loaded { expanded_paths, .. }
                | AppState::Loading { expanded_paths, .. } = &mut self.state
                {
                    if !expanded_paths.insert(path.clone()) {
                        expanded_paths.remove(&path);
                    }
                }
                Task::none()
            }
            Message::Navigate(path) => {
                if let AppState::Loaded {
                    root_node,
                    current_path,
                    path_history,
                    expanded_paths,
                    ..
                } = &mut self.state
                {
                    if let Some(node) = Self::find_node(root_node, &path) {
                        if node.is_dir {
                            path_history.push(current_path.clone());
                            *current_path = path;
                            expanded_paths.clear();
                        }
                    }
                }
                Task::none()
            }
            Message::GoBack => {
                if let AppState::Loaded {
                    current_path,
                    path_history,
                    expanded_paths,
                    ..
                } = &mut self.state
                {
                    if let Some(prev) = path_history.pop() {
                        *current_path = prev;
                        expanded_paths.clear();
                    }
                }
                Task::none()
            }
            Message::OpenInExplorer(path) => {
                std::process::Command::new("explorer")
                    .arg("/select,")
                    .arg(&path)
                    .spawn()
                    .ok();
                Task::none()
            }
            Message::RequestDelete(path) => {
                let path_clone = path.clone();
                Task::perform(
                    async move {
                        let success = get_tokio_runtime()
                            .spawn_blocking(move || trash::delete(&path_clone).is_ok())
                            .await
                            .unwrap_or(false);

                        (path, success)
                    },
                    |(p, s)| Message::FileDeleted(p, s),
                )
            }
            Message::FileDeleted(path, success) => {
                if success {
                    if let AppState::Loaded { root_node, .. } = &mut self.state {
                        root_node.remove_by_path(&path);
                    }
                } else {
                    println!("放入回收站失败");
                }
                Task::none()
            }
            Message::ConfirmRename => {
                if let AppState::Loaded {
                    root_node,
                    rename_target,
                    ..
                } = &mut self.state
                {
                    if let Some((target_path, new_name)) = rename_target.take() {
                        let p = Path::new(&target_path);
                        if let Some(parent) = p.parent() {
                            let new_path = parent.join(&new_name);

                            if std::fs::rename(p, &new_path).is_ok() {
                                root_node.rename_by_path(&target_path, &new_name);
                            } else {
                                println!("rename failed，pls check file 有没有被占用");
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::RenameInputChanged(val) => {
                if let AppState::Loaded {
                    rename_target: Some((_, input)),
                    ..
                } = &mut self.state
                {
                    *input = val;
                }
                Task::none()
            }

            Message::CancelRename => {
                if let AppState::Loaded { rename_target, .. } = &mut self.state {
                    *rename_target = None;
                }
                Task::none()
            }
            Message::RequestRename(path) => {
                if let AppState::Loaded { rename_target, .. } = &mut self.state {
                    let old_name = Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    *rename_target = Some((path, old_name));
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.state {
            AppState::Idle => container(
                column![
                    text("正在自动定位系统盘 MFT...").size(20),
                    button(text("改为选择 Everything 导出的 CSV"))
                        .style(button_style)
                        .padding(Padding {
                            top: 8.0,
                            right: 12.0,
                            bottom: 8.0,
                            left: 12.0,
                        })
                        .on_press(Message::PickCsvFile)
                ]
                .spacing(16),
            )
            .width(Fill)
            .height(Fill)
            .center(Fill)
            .into(),

            AppState::Loading {
                source_kind,
                source_path,
                root_node,
                current_path,
                expanded_paths,
                ..
            } => {
                let current_node = Self::find_node(root_node, current_path).unwrap_or(root_node);

                let top_bar = row![
                    Circular::new().size(20.0).easing(&easing::STANDARD),
                    text(format!(
                        "少女祈祷中... 来源: {} | 已索引: {} | {}",
                        source_kind.label(),
                        treemap::format_size(root_node.size),
                        source_path
                    ))
                    .size(18)
                ]
                .spacing(15)
                .align_y(iced::Alignment::Center);

                let map = TreemapCanvas {
                    root_node: current_node,
                    expanded_paths,
                };

                container(column![
                    container(top_bar).padding(10),
                    Canvas::new(map).width(Fill).height(Fill)
                ])
                .width(Fill)
                .height(Fill)
                .into()
            }

            AppState::Error(err) => container(
                column![
                    text(format!("ERR: {}", err)).style(text::danger),
                    row![
                        button("重试自动加载系统盘 MFT")
                            .style(button_style)
                            .on_press(Message::AutoLoadSystemMft),
                        button("选择 Everything 导出的 CSV")
                            .style(button_style)
                            .on_press(Message::PickCsvFile),
                    ]
                    .spacing(12)
                ]
                .spacing(16),
            )
            .width(Fill)
            .height(Fill)
            .center(Fill)
            .into(),

            AppState::Loaded {
                source_kind,
                source_path,
                root_node,
                current_path,
                path_history,
                expanded_paths,
                rename_target,
                ..
            } => {
                let current_node = Self::find_node(root_node, current_path).unwrap_or(root_node);

                let mut top_bar = row![].spacing(10).align_y(iced::Alignment::Center);
                if !path_history.is_empty() {
                    top_bar = top_bar.push(button("折返").style(button_style).on_press(Message::GoBack));
                }

                let display_path = if current_path.is_empty() {
                    "计算机 (根目录)"
                } else {
                    current_path
                };
                top_bar = top_bar.push(
                    text(format!(
                        "当前位置: {} ({}) | 来源: {} | {}",
                        display_path,
                        treemap::format_size(current_node.size),
                        source_kind.label(),
                        source_path
                    ))
                    .size(18),
                );

                if let Some((_, input_name)) = rename_target {
                    top_bar = top_bar
                        .push(text("正在重命名:"))
                        .push(
                            text_input("输入新文件名...", input_name)
                                .on_input(Message::RenameInputChanged)
                                .width(200.0)
                                .on_submit(Message::ConfirmRename),
                        )
                        .push(
                            button("OK")
                                .style(button::success)
                                .on_press(Message::ConfirmRename),
                        )
                        .push(
                            button("Cancel")
                                .style(button::danger)
                                .on_press(Message::CancelRename),
                        );
                }

                let map = TreemapCanvas {
                    root_node: current_node,
                    expanded_paths,
                };

                container(column![
                    container(top_bar).padding(10),
                    Canvas::new(map).width(Fill).height(Fill)
                ])
                .style(succeed_container_style)
                .width(Fill)
                .height(Fill)
                .into()
            }
        }
    }
}
