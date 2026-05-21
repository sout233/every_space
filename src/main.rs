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

use crate::theme::{button_style, succeed_container_style, idle_container_style, recommend_button_style, secondary_button_style};

#[allow(dead_code)]
fn get_tokio_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("init tokio runtime failed")
    })
}

fn init_logger() {
    use std::io::Write;
    env_logger::Builder::new()
        .format(|buf, record| {
            writeln!(buf, "{}", record.args())
        })
        .filter(None, log::LevelFilter::Info)
        .init();
}

pub fn main() -> iced::Result {
    init_logger();
    log::info!("[app] app started");
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
    LoadEverything,
    LoadMft,
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
    BackToIdle,
}

enum AppState {
    Idle,
    Loading {
        source_kind: DataSourceKind,
        source_path: String,
        root_node: FileNode,
        current_path: String,
        path_history: Vec<String>,
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
        rename_target: Option<(String, String)>, // (old path, new name input)
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
    #[allow(dead_code)]
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
            path_history: Vec::new(),
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
            Message::LoadEverything => {
                log::info!("[app] loading everything dll source");
                let system_drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
                self.start_loading(DataSourceKind::EverythingDll, system_drive)
            }
            Message::LoadMft => {
                log::info!("[app] loading mft source");
                match Self::resolve_system_mft_path() {
                    Ok(path) => self.start_loading(DataSourceKind::Mft, path),
                    Err(err) => {
                        log::error!("[app] resolve mft path err: {}", err);
                        self.state = AppState::Error(err);
                        Task::none()
                    }
                }
            }
            Message::PickCsvFile => {
                log::info!("[app] opening csv picker");
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("CSV Data", &["csv"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_string_lossy().to_string())
                    },
                    |path| Message::FilePicked(DataSourceKind::Csv, path),
                )
            }
            Message::FilePicked(source_kind, Some(path)) => {
                log::info!("[app] selected source {:?} at {}", source_kind, path);
                self.start_loading(source_kind, path)
            }
            Message::FilePicked(_, None) => {
                log::info!("[app] csv selection canceled");
                Task::none()
            }
            Message::Tick(_now) => {
                let mut is_finished_loading = false;
                let mut error_msg = None;

                if let AppState::Loading {
                    root_node,
                    rx,
                    anim_tick,
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
                    log::error!("[app] err occurred during load: {}", err);
                    self.state = AppState::Error(err);
                    return Task::none();
                }

                if is_finished_loading {
                    log::info!("[app] 就此结束了吗……");
                    let mut temp_state = AppState::Idle;
                    std::mem::swap(&mut self.state, &mut temp_state);

                    if let AppState::Loading {
                        source_kind,
                        source_path,
                        root_node,
                        current_path,
                        path_history,
                        expanded_paths,
                        ..
                    } = temp_state
                    {
                        self.state = AppState::Loaded {
                            source_kind,
                            source_path,
                            root_node,
                            current_path,
                            path_history,
                            expanded_paths,
                            anim_tick: 0,
                            rename_target: None,
                        };
                    }
                }
                Task::none()
            }
            Message::ToggleExpand(path) => {
                log::info!("[app] toggle expand {}", path);
                match &mut self.state {
                    AppState::Loading { expanded_paths, .. } | AppState::Loaded { expanded_paths, .. } => {
                        if expanded_paths.contains(&path) {
                            expanded_paths.remove(&path);
                        } else {
                            expanded_paths.insert(path);
                        }
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::Navigate(path) => {
                log::info!("[app] navigate to {}", path);
                match &mut self.state {
                    AppState::Loading {
                        root_node,
                        current_path,
                        path_history,
                        ..
                    }
                    | AppState::Loaded {
                        root_node,
                        current_path,
                        path_history,
                        ..
                    } => {
                        if let Some(node) = Self::find_node(root_node, &path) {
                            if node.is_dir {
                                path_history.push(current_path.clone());
                                *current_path = path;
                            }
                        }
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::GoBack => {
                log::info!("[app] go back");
                match &mut self.state {
                    AppState::Loading {
                        current_path,
                        path_history,
                        ..
                    }
                    | AppState::Loaded {
                        current_path,
                        path_history,
                        ..
                    } => {
                        if let Some(prev) = path_history.pop() {
                            *current_path = prev;
                        }
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::OpenInExplorer(path) => {
                log::info!("[app] open explorer select {}", path);
                let _ = std::process::Command::new("explorer")
                    .arg("/select,")
                    .arg(&path)
                    .spawn();
                Task::none()
            }
            Message::RequestDelete(path) => {
                log::info!("[app] request delete {}", path);
                let path_clone = path.clone();
                Task::perform(
                    async move {
                        let p = std::path::Path::new(&path_clone);
                        trash::delete(p).is_ok()
                    },
                    move |success| Message::FileDeleted(path, success)
                )
            }
            Message::FileDeleted(path, success) => {
                if success {
                    log::info!("[app] delete success {}", path);
                    if let AppState::Loaded { root_node, .. } = &mut self.state {
                        root_node.remove_by_path(&path);
                    }
                } else {
                    log::error!("[app] delete failed {}", path);
                }
                Task::none()
            }
            Message::RequestRename(path) => {
                log::info!("[app] request rename old {}", path);
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
                                log::info!("[app] file rename finished");
                                root_node.rename_by_path(&target_path, &new_name);
                            } else {
                                log::error!("[app] rename failed");
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::CancelRename => {
                log::info!("[app] rename canceled");
                if let AppState::Loaded { rename_target, .. } = &mut self.state {
                    *rename_target = None;
                }
                Task::none()
            }
            Message::BackToIdle => {
                log::info!("[app] back to main menu");
                self.state = AppState::Idle;
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.state {
            AppState::Idle => {
                let title_section = column![
                    text("EverySpace")
                        .size(48),
                    text("硬盘被橄榄了……")
                        .size(18)
                        .style(text::secondary),
                ]
                .spacing(12)
                .align_x(iced::Alignment::Center);

                let cards = row![
                    button(
                        column![
                            text("直接调用 Everything")
                                .size(20),
                            text("推荐")
                                .size(13),
                            text("这个好，用这个")
                                .size(12),
                            text("（要求 Everything 正在后台运行）")
                                .size(11),
                        ]
                        .spacing(8)
                        .align_x(iced::Alignment::Center)
                    )
                    .style(recommend_button_style)
                    .padding(24)
                    .on_press(Message::LoadEverything)
                    .width(235),

                    button(
                        column![
                            text("读取 Everything CSV 文件")
                                .size(20),
                            text("离线分析")
                                .size(13),
                            text("读取手动导出的 CSV，无法热更新内容")
                                .size(12),
                            text("（明明我才是先来的……）")
                                .size(11),
                        ]
                        .spacing(8)
                        .align_x(iced::Alignment::Center)
                    )
                    .style(secondary_button_style)
                    .padding(24)
                    .on_press(Message::PickCsvFile)
                    .width(275),

                    button(
                        column![
                            text("直接读取 NTFS MFT")
                                .size(20),
                            text("我操。NTFS。")
                                .size(13),
                            text("不太稳，不太准")
                                .size(12),
                            text("（需要右键以管理员权限运行）")
                                .size(11),
                        ]
                        .spacing(8)
                        .align_x(iced::Alignment::Center)
                    )
                    .style(secondary_button_style)
                    .padding(24)
                    .on_press(Message::LoadMft)
                    .width(230),
                ]
                .spacing(24)
                .align_y(iced::Alignment::Center);

                container(
                    column![title_section, cards]
                        .spacing(48)
                        .align_x(iced::Alignment::Center)
                )
                .style(idle_container_style)
                .width(Fill)
                .height(Fill)
                .center(Fill)
                .into()
            }

            AppState::Loading {
                source_kind,
                source_path,
                root_node,
                current_path,
                path_history,
                expanded_paths,
                ..
            } => {
                let current_node = Self::find_node(root_node, current_path).unwrap_or(root_node);

                let mut top_bar = row![
                    Circular::new().size(20.0).easing(&easing::STANDARD),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center);

                if !path_history.is_empty() {
                    top_bar = top_bar.push(
                        button("返回上级")
                            .style(button_style)
                            .padding(Padding {
                                top: 6.0,
                                right: 12.0,
                                bottom: 6.0,
                                left: 12.0,
                            })
                            .on_press(Message::GoBack)
                    );
                }

                let display_path = if current_path.is_empty() {
                    "计算机 (根目录)"
                } else {
                    current_path
                };

                top_bar = top_bar.push(
                    text(format!(
                        "实时扫描中... 当前位置: {} (已阅: {}) | 来源: {} ({}) | 已分析总量: {}",
                        display_path,
                        treemap::format_size(current_node.size),
                        source_kind.label(),
                        source_path,
                        treemap::format_size(root_node.size)
                    ))
                    .size(16),
                );

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

            AppState::Error(err) => container(
                column![
                    text("出错了")
                        .size(36),
                    text(format!("错误信息: {}", err))
                        .size(16)
                        .style(text::danger),
                    button("返回主选择界面")
                        .style(button_style)
                        .padding(Padding {
                            top: 8.0,
                            right: 12.0,
                            bottom: 8.0,
                            left: 12.0,
                        })
                        .on_press(Message::BackToIdle)
                ]
                .spacing(24)
                .align_x(iced::Alignment::Center),
            )
            .style(idle_container_style)
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

                let mut top_bar = row![].spacing(12).align_y(iced::Alignment::Center);
                if !path_history.is_empty() {
                    top_bar = top_bar.push(
                        button("返回上级")
                            .style(button_style)
                            .padding(Padding {
                                top: 6.0,
                                right: 12.0,
                                bottom: 6.0,
                                left: 12.0,
                            })
                            .on_press(Message::GoBack)
                    );
                }

                top_bar = top_bar.push(
                    button("重新选择数据源")
                        .style(button_style)
                        .padding(Padding {
                            top: 6.0,
                            right: 12.0,
                            bottom: 6.0,
                            left: 12.0,
                        })
                        .on_press(Message::BackToIdle)
                );

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
                    .size(16),
                );

                if let Some((_, input_name)) = rename_target {
                    top_bar = top_bar
                        .push(text("重命名为:").size(14))
                        .push(
                            text_input("输入新名称...", input_name)
                                .on_input(Message::RenameInputChanged)
                                .width(200.0)
                                .on_submit(Message::ConfirmRename),
                        )
                        .push(
                            button("确定")
                                .style(button::success)
                                .padding(Padding {
                                    top: 4.0,
                                    right: 10.0,
                                    bottom: 4.0,
                                    left: 10.0,
                                })
                                .on_press(Message::ConfirmRename),
                        )
                        .push(
                            button("取消")
                                .style(button::danger)
                                .padding(Padding {
                                    top: 4.0,
                                    right: 10.0,
                                    bottom: 4.0,
                                    left: 10.0,
                                })
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
