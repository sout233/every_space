use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::fs::File;
use std::sync::mpsc::Sender;

const STREAM_CHUNK_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSourceKind {
    EverythingDll,
    Csv,
}

impl DataSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::EverythingDll => "Everything IPC",
            Self::Csv => "Everything CSV",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EverythingRecord {
    #[serde(rename = "名称")]
    pub name: String,
    #[serde(rename = "路径")]
    pub path: String,
    #[serde(rename = "大小")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub children: FxHashMap<String, FileNode>,
}

impl FileNode {
    pub fn new(name: String, full_path: String, is_dir: bool) -> Self {
        Self {
            name,
            full_path,
            is_dir,
            size: 0,
            children: FxHashMap::default(),
        }
    }

    pub fn root() -> Self {
        Self::new("Computer".into(), "".into(), true)
    }

    pub fn remove_by_path(&mut self, target_path: &str) -> Option<u64> {
        let parts: Vec<&str> = target_path.split('\\').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return None;
        }
        self.remove_recursive(&parts)
    }

    fn remove_recursive(&mut self, parts: &[&str]) -> Option<u64> {
        if parts.len() == 1 {
            if let Some(removed) = self.children.remove(parts[0]) {
                self.size = self.size.saturating_sub(removed.size);
                return Some(removed.size);
            }
            return None;
        }
        if let Some(child) = self.children.get_mut(parts[0]) {
            if let Some(removed_size) = child.remove_recursive(&parts[1..]) {
                self.size = self.size.saturating_sub(removed_size);
                return Some(removed_size);
            }
        }
        None
    }

    pub fn rename_by_path(&mut self, target_path: &str, new_name: &str) -> bool {
        let parts: Vec<&str> = target_path.split('\\').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return false;
        }
        self.rename_recursive(&parts, new_name)
    }

    fn rename_recursive(&mut self, parts: &[&str], new_name: &str) -> bool {
        if parts.len() == 1 {
            if let Some(mut node) = self.children.remove(parts[0]) {
                node.name = new_name.to_string();

                let parent_path = if let Some(idx) = node.full_path.rfind('\\') {
                    &node.full_path[..idx]
                } else {
                    ""
                };
                node.full_path = if parent_path.is_empty() {
                    new_name.to_string()
                } else {
                    format!("{}\\{}", parent_path, new_name)
                };

                self.children.insert(new_name.to_string(), node);
                return true;
            }
            return false;
        }
        if let Some(child) = self.children.get_mut(parts[0]) {
            return child.rename_recursive(&parts[1..], new_name);
        }
        false
    }
}

#[derive(Debug, Clone)]
struct FlatNode {
    parent_path: String,
    name: String,
    full_path: String,
    is_dir: bool,
    size: u64,
}

struct TreeAccumulator {
    root: FileNode,
    pending_count: usize,
}

impl TreeAccumulator {
    fn new() -> Self {
        Self {
            root: FileNode::root(),
            pending_count: 0,
        }
    }

    fn push_node(&mut self, node: FlatNode) {
        self.insert_ancestors(&node.parent_path, node.size);
        self.insert_leaf(node);
        self.pending_count += 1;
    }

    fn should_flush(&self) -> bool {
        self.pending_count >= STREAM_CHUNK_SIZE
    }

    fn take_root(&mut self) -> FileNode {
        self.pending_count = 0;
        std::mem::replace(&mut self.root, FileNode::root())
    }

    fn insert_ancestors(&mut self, parent_path: &str, item_size: u64) {
        let mut current = &mut self.root;
        current.size += item_size;

        let mut current_path = String::new();
        for part in parent_path.split('\\').filter(|s| !s.is_empty()) {
            if current_path.is_empty() {
                current_path.push_str(part);
            } else {
                current_path.push('\\');
                current_path.push_str(part);
            }

            let next = current
                .children
                .entry(part.to_string())
                .or_insert_with(|| FileNode::new(part.to_string(), current_path.clone(), true));
            next.is_dir = true;
            next.size += item_size;
            current = next;
        }
    }

    fn insert_leaf(&mut self, node: FlatNode) {
        let mut current = &mut self.root;
        for part in node.parent_path.split('\\').filter(|s| !s.is_empty()) {
            current = current
                .children
                .entry(part.to_string())
                .or_insert_with(|| FileNode::new(part.to_string(), part.to_string(), true));
        }

        let target = current.children.entry(node.name.clone()).or_insert_with(|| {
            FileNode::new(node.name.clone(), node.full_path.clone(), node.is_dir)
        });
        target.full_path = node.full_path;
        target.is_dir = node.is_dir;
        target.size += node.size;
    }
}

pub fn merge_into(a: &mut FileNode, b: FileNode) {
    a.size += b.size;
    a.is_dir = a.is_dir && b.is_dir;

    for (k, vb) in b.children {
        match a.children.entry(k) {
            std::collections::hash_map::Entry::Occupied(mut o) => {
                merge_into(o.get_mut(), vb);
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(vb);
            }
        }
    }
}

pub fn build_tree_stream(
    source_kind: DataSourceKind,
    file_path: String,
    tx: Sender<Result<FileNode, String>>,
) {
    log::info!("[app] 世界树正在天上失禁地看着你……");

    std::thread::spawn(move || {
        let result = match source_kind {
            DataSourceKind::EverythingDll => build_tree_from_everything_dll(&file_path, &tx),
            DataSourceKind::Csv => build_tree_from_csv(&file_path, &tx),
        };

        if let Err(err) = result {
            let _ = tx.send(Err(err));
        }
    });
}

fn build_tree_from_csv(file_path: &str, tx: &Sender<Result<FileNode, String>>) -> Result<(), String> {
    log::info!("[csv] reading csv file");
    let file = File::open(file_path).map_err(|e| format!("打开 CSV 失败: {e}"))?;
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(file);

    let mut acc = TreeAccumulator::new();
    for result in rdr.deserialize::<EverythingRecord>() {
        let record = result.map_err(|e| format!("解析 CSV 失败: {e}"))?;
        let item_size = record.size.unwrap_or(0);
        let is_dir = record.size.is_none();
        let parent_path = normalize_windows_str_path(&record.path);
        let full_path = join_windows_path(&parent_path, &record.name);

        acc.push_node(FlatNode {
            parent_path,
            name: record.name,
            full_path,
            is_dir,
            size: item_size,
        });

        if acc.should_flush() {
            flush_partial_tree(&mut acc, tx)?;
        }
    }

    flush_partial_tree(&mut acc, tx)?;
    log::info!("[csv] csv deserialized successfully");
    Ok(())
}


fn flush_partial_tree(
    acc: &mut TreeAccumulator,
    tx: &Sender<Result<FileNode, String>>,
) -> Result<(), String> {
    if acc.pending_count == 0 {
        return Ok(());
    }

    tx.send(Ok(acc.take_root()))
        .map_err(|_| "加载已取消".to_string())
}

fn join_windows_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}\\{name}")
    }
}


fn normalize_windows_str_path(path: &str) -> String {
    path.split('\\')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}


#[cfg(windows)]
pub struct EverythingLib {
    hmodule: windows_sys::Win32::Foundation::HMODULE,
    pub set_search: unsafe extern "system" fn(*const u16),
    pub set_request_flags: unsafe extern "system" fn(u32),
    pub query: unsafe extern "system" fn(i32) -> i32,
    pub get_num_results: unsafe extern "system" fn() -> u32,
    pub is_folder_result: unsafe extern "system" fn(u32) -> i32,
    pub get_result_file_name: unsafe extern "system" fn(u32) -> *const u16,
    pub get_result_path: unsafe extern "system" fn(u32) -> *const u16,
    pub get_result_size: unsafe extern "system" fn(u32, *mut u64) -> i32,
    pub reset: unsafe extern "system" fn(),
    pub get_last_error: unsafe extern "system" fn() -> u32,
}

#[cfg(windows)]
unsafe extern "system" {
    fn FreeLibrary(hLibModule: windows_sys::Win32::Foundation::HMODULE) -> i32;
}


#[cfg(windows)]
const EVERYTHING64_DLL_BYTES: &[u8] = include_bytes!("../sdk/dll/Everything64.dll");

#[cfg(windows)]
impl EverythingLib {
    fn extract_embedded_dll() -> Result<std::path::PathBuf, String> {
        log::info!("[dll] extracting embedded dll");
        // 1. try extracting to same folder as exe
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        if let Some(mut path) = exe_dir {
            path.push("Everything64.dll");
            if !path.exists() {
                if std::fs::write(&path, EVERYTHING64_DLL_BYTES).is_ok() {
                    return Ok(path);
                }
            } else {
                return Ok(path);
            }
        }

        // 2. fallback to current working directory
        let mut work_path = std::env::current_dir()
            .map_err(|e| format!("获取当前工作目录失败: {}", e))?;
        work_path.push("Everything64.dll");

        if !work_path.exists() {
            std::fs::write(&work_path, EVERYTHING64_DLL_BYTES)
                .map_err(|e| format!("释放 Everything64.dll 到工作目录失败: {}", e))?;
        }
        Ok(work_path)
    }

    pub fn load() -> Result<Self, String> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};

        log::info!("[dll] trying to load dll");
        let paths = [
            std::path::Path::new("Everything64.dll").to_path_buf(),
            std::path::Path::new("sdk/dll/Everything64.dll").to_path_buf(),
            std::path::Path::new("sdk\\dll\\Everything64.dll").to_path_buf(),
        ];

        let mut hmodule = std::ptr::null_mut();
        for path in &paths {
            if path.exists() {
                let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
                let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
                if !handle.is_null() {
                    hmodule = handle;
                    break;
                }
            }
        }

        if hmodule.is_null() {
            let wide: Vec<u16> = OsStr::new("Everything64.dll").encode_wide().chain(std::iter::once(0)).collect();
            let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
            if !handle.is_null() {
                hmodule = handle;
            }
        }

        // extract and load dll if not found
        if hmodule.is_null() {
            if let Ok(extracted_path) = Self::extract_embedded_dll() {
                let wide: Vec<u16> = OsStr::new(&extracted_path).encode_wide().chain(std::iter::once(0)).collect();
                let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
                if !handle.is_null() {
                    hmodule = handle;
                }
            }
        }

        if hmodule.is_null() {
            return Err("未找到 Everything64.dll 且释放嵌入 DLL 失败。请确保程序对运行目录或工作目录具有写入权限。".to_string());
        }

        log::info!("[dll] dll loaded successfully");

        unsafe {
            let get_fn = |name: &str| -> Result<*const std::ffi::c_void, String> {
                let name_c = std::ffi::CString::new(name).unwrap();
                let addr = GetProcAddress(hmodule, name_c.as_ptr() as *const u8);
                if addr.is_none() {
                    FreeLibrary(hmodule);
                    return Err(format!("在 DLL 中未找到函数: {}", name));
                }
                Ok(addr.unwrap() as *const std::ffi::c_void)
            };

            Ok(Self {
                hmodule,
                set_search: std::mem::transmute(get_fn("Everything_SetSearchW")?),
                set_request_flags: std::mem::transmute(get_fn("Everything_SetRequestFlags")?),
                query: std::mem::transmute(get_fn("Everything_QueryW")?),
                get_num_results: std::mem::transmute(get_fn("Everything_GetNumResults")?),
                is_folder_result: std::mem::transmute(get_fn("Everything_IsFolderResult")?),
                get_result_file_name: std::mem::transmute(get_fn("Everything_GetResultFileNameW")?),
                get_result_path: std::mem::transmute(get_fn("Everything_GetResultPathW")?),
                get_result_size: std::mem::transmute(get_fn("Everything_GetResultSize")?),
                reset: std::mem::transmute(get_fn("Everything_Reset")?),
                get_last_error: std::mem::transmute(get_fn("Everything_GetLastError")?),
            })
        }
    }
}

#[cfg(windows)]
impl Drop for EverythingLib {
    fn drop(&mut self) {
        if !self.hmodule.is_null() {
            unsafe {
                FreeLibrary(self.hmodule);
            }
        }
    }
}


#[cfg(windows)]
unsafe fn u16_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}


#[cfg(windows)]
fn build_tree_from_everything_dll(
    search_str: &str,
    tx: &Sender<Result<FileNode, String>>,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    let lib = EverythingLib::load()?;
    let search_wide: Vec<u16> = std::ffi::OsStr::new(search_str)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        log::info!("[everything] querying items");
        (lib.set_search)(search_wide.as_ptr());
        // request name, path, and size
        (lib.set_request_flags)(0x00000001 | 0x00000002 | 0x00000010);

        let ok = (lib.query)(1);
        if ok == 0 {
            let err_code = (lib.get_last_error)();
            if err_code == 2 {
                return Err("无法连接到 Everything 服务。请确保 Everything 客户端已启动且正在后台运行。".to_string());
            } else {
                return Err(format!("Everything 查询失败，错误码: {}", err_code));
            }
        }

        let num_results = (lib.get_num_results)();
        log::info!("[everything] query success, found {} items", num_results);
        let mut acc = TreeAccumulator::new();

        for i in 0..num_results {
            let is_dir = (lib.is_folder_result)(i) != 0;

            let name_ptr = (lib.get_result_file_name)(i);
            let path_ptr = (lib.get_result_path)(i);

            let name = u16_ptr_to_string(name_ptr);
            let parent_path = u16_ptr_to_string(path_ptr);

            let mut size = 0u64;
            if !is_dir {
                (lib.get_result_size)(i, &mut size);
            }

            let parent_path_norm = normalize_windows_str_path(&parent_path);
            let full_path = join_windows_path(&parent_path_norm, &name);

            acc.push_node(FlatNode {
                parent_path: parent_path_norm,
                name,
                full_path,
                is_dir,
                size,
            });

            if acc.should_flush() {
                flush_partial_tree(&mut acc, tx)?;
            }
        }

        flush_partial_tree(&mut acc, tx)?;
        (lib.reset)();
    }

    Ok(())
}

#[cfg(not(windows))]
fn build_tree_from_everything_dll(
    _search_str: &str,
    _tx: &Sender<Result<FileNode, String>>,
) -> Result<(), String> {
    Err("Everything API 仅支持 Windows 系统".to_string())
}
