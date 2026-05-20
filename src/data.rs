use mft::MftParser;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path};
use std::sync::mpsc::Sender;

const STREAM_CHUNK_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSourceKind {
    Csv,
    Mft,
}

impl DataSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "Everything CSV",
            Self::Mft => "NTFS MFT",
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
    std::thread::spawn(move || {
        let result = match source_kind {
            DataSourceKind::Csv => build_tree_from_csv(&file_path, &tx),
            DataSourceKind::Mft => build_tree_from_mft(&file_path, &tx),
        };

        if let Err(err) = result {
            let _ = tx.send(Err(err));
        }
    });
}

fn build_tree_from_csv(file_path: &str, tx: &Sender<Result<FileNode, String>>) -> Result<(), String> {
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
    Ok(())
}

fn build_tree_from_mft(file_path: &str, tx: &Sender<Result<FileNode, String>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        return build_tree_from_volume_mft(file_path, tx);
    }

    #[cfg(not(windows))]
    {
        let mut parser = open_mft_parser(file_path)?;
        let mut acc = TreeAccumulator::new();

        let entry_count = parser.get_entry_count();
        for entry_id in 0..entry_count {
            let entry = match parser.get_entry(entry_id) {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if !entry.header.is_valid() || !entry.is_allocated() {
                continue;
            }

            let is_dir = entry.is_dir();
            let size = if is_dir {
                0
            } else {
                entry
                    .find_best_name_attribute()
                    .map(|attr| attr.logical_size)
                    .unwrap_or(0)
            };
            let Some(full_path_buf) = parser
                .get_full_path_for_entry(&entry)
                .map_err(|e| format!("解析 MFT 路径失败: {e}"))?
            else {
                continue;
            };

            let normalized = normalize_mft_path(&full_path_buf);
            if normalized.is_empty() {
                continue;
            }

            let Some((parent_path, name)) = split_windows_path(&normalized) else {
                continue;
            };

            acc.push_node(FlatNode {
                parent_path,
                name,
                full_path: normalized,
                is_dir,
                size,
            });

            if acc.should_flush() {
                flush_partial_tree(&mut acc, tx)?;
            }
        }

        flush_partial_tree(&mut acc, tx)?;
        Ok(())
    }
}

#[allow(dead_code)]
fn open_mft_parser(file_path: &str) -> Result<MftParser<BufReader<File>>, String> {
    let file = open_mft_file(file_path)?;
    let size = file
        .metadata()
        .map(|metadata| metadata.len())
        .ok();

    MftParser::from_read_seek(BufReader::with_capacity(1024 * 1024, file), size)
        .map_err(|e| format!("打开 MFT 失败: {e}"))
}

#[cfg(windows)]
#[allow(dead_code)]
fn open_mft_file(file_path: &str) -> Result<File, String> {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LUID,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
        SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn to_utf16(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe fn enable_backup_privilege() -> Result<(), String> {
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(format!(
                "OpenProcessToken 失败，os err {}",
                unsafe { GetLastError() }
            ));
        }

        let privilege_name = to_utf16("SeBackupPrivilege");
        let mut luid: LUID = unsafe { zeroed() };
        if unsafe { LookupPrivilegeValueW(std::ptr::null(), privilege_name.as_ptr(), &mut luid) } == 0
        {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(token) };
            return Err(format!("LookupPrivilegeValueW 失败，os err {}", err));
        }

        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        if unsafe {
            AdjustTokenPrivileges(
                token,
                0,
                &privileges,
                size_of::<TOKEN_PRIVILEGES>() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(token) };
            return Err(format!("AdjustTokenPrivileges 失败，os err {}", err));
        }

        let err = unsafe { GetLastError() };
        unsafe { CloseHandle(token) };
        if err != 0 {
            return Err(format!("启用 SeBackupPrivilege 失败，os err {}", err));
        }

        Ok(())
    }

    unsafe {
        enable_backup_privilege()?;

        let wide_path = to_utf16(file_path);
        let handle = CreateFileW(
            wide_path.as_ptr(),
            0x80000000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_SEQUENTIAL_SCAN,
            std::ptr::null_mut(),
        );

        if handle == INVALID_HANDLE_VALUE {
            let err = GetLastError();
            let mut msg = format!("打开 MFT 失败，os err {}。目标路径: {}。", err, file_path);
            if err == 5 {
                msg.push_str(" 拒绝访问 (Error 5)。读取 NTFS 元文件需要管理员权限，请确保以管理员身份运行此程序。");
            }
            return Err(msg);
        }

        Ok(File::from_raw_handle(handle as _))
    }
}

#[cfg(windows)]
fn build_tree_from_volume_mft(
    file_path: &str,
    tx: &Sender<Result<FileNode, String>>,
) -> Result<(), String> {
    use mft::MftEntry;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_GET_NTFS_FILE_RECORD, FSCTL_GET_NTFS_VOLUME_DATA, NTFS_FILE_RECORD_INPUT_BUFFER,
        NTFS_FILE_RECORD_OUTPUT_BUFFER, NTFS_VOLUME_DATA_BUFFER,
    };

    fn to_utf16(value: &str) -> Vec<u16> {
        OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn extract_drive_root(file_path: &str) -> Result<String, String> {
        let drive = file_path
            .chars()
            .take(2)
            .collect::<String>();
        if drive.len() != 2 || !drive.ends_with(':') {
            return Err(format!("无法从路径推断卷名: {file_path}"));
        }
        Ok(format!("\\\\.\\{drive}"))
    }

    unsafe fn get_volume_data_info(handle: HANDLE) -> Result<(u32, u64), String> {
        let mut volume_data = NTFS_VOLUME_DATA_BUFFER {
            VolumeSerialNumber: 0,
            NumberSectors: 0,
            TotalClusters: 0,
            FreeClusters: 0,
            TotalReserved: 0,
            BytesPerSector: 0,
            BytesPerCluster: 0,
            BytesPerFileRecordSegment: 0,
            ClustersPerFileRecordSegment: 0,
            MftValidDataLength: 0,
            MftStartLcn: 0,
            Mft2StartLcn: 0,
            MftZoneStart: 0,
            MftZoneEnd: 0,
        };
        let mut returned = 0u32;

        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                std::ptr::null(),
                0,
                &mut volume_data as *mut _ as *mut _,
                std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!(
                "读取 NTFS 卷信息失败，os err {}",
                unsafe { GetLastError() }
            ));
        }

        let record_size = volume_data.BytesPerFileRecordSegment;
        let total_records = volume_data.MftValidDataLength / record_size as i64;
        Ok((record_size, total_records as u64))
    }

    unsafe fn read_record(
        handle: HANDLE,
        file_ref: i64,
        record_size: usize,
    ) -> Result<Option<(u64, Vec<u8>)>, String> {
        let input = NTFS_FILE_RECORD_INPUT_BUFFER {
            FileReferenceNumber: file_ref,
        };
        let out_size =
            std::mem::size_of::<NTFS_FILE_RECORD_OUTPUT_BUFFER>() + record_size.saturating_sub(1);
        let mut out_buf = vec![0u8; out_size];
        let mut returned = 0u32;

        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_NTFS_FILE_RECORD,
                &input as *const _ as *const _,
                std::mem::size_of::<NTFS_FILE_RECORD_INPUT_BUFFER>() as u32,
                out_buf.as_mut_ptr() as *mut _,
                out_size as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            let err = unsafe { GetLastError() };
            const ERROR_HANDLE_EOF: u32 = 38;
            if err == ERROR_HANDLE_EOF {
                return Ok(None);
            }
            return Err(format!(
                "读取 NTFS file record 失败，os err {}，请求 record {}",
                err, file_ref
            ));
        }

        let header = unsafe { &*(out_buf.as_ptr() as *const NTFS_FILE_RECORD_OUTPUT_BUFFER) };
        let file_record_length = header.FileRecordLength as usize;
        let file_record_number = (header.FileReferenceNumber as u64) & 0x0000FFFFFFFFFFFFu64;
        let start = std::mem::size_of::<i64>() + std::mem::size_of::<u32>();
        let end = start + file_record_length;
        if end > out_buf.len() {
            return Err(format!(
                "NTFS file record 缓冲区长度异常: record={}, len={}, out={}",
                file_record_number, file_record_length, out_buf.len()
            ));
        }

        Ok(Some((file_record_number, out_buf[start..end].to_vec())))
    }

    #[derive(Debug, Clone)]
    struct TempMftEntry {
        parent_id: u64,
        name: String,
        is_dir: bool,
        size: u64,
    }

    let volume_path = extract_drive_root(file_path)?;
    let wide_volume_path = to_utf16(&volume_path);

    let handle = unsafe {
        CreateFileW(
            wide_volume_path.as_ptr(),
            FILE_GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        let err = unsafe { GetLastError() };
        let mut msg = format!("打开 NTFS 卷失败，os err {}。目标卷: {}", err, volume_path);
        if err == 5 {
            msg.push_str(" 拒绝访问 (Error 5)。读取底层 MFT 需要管理员权限。请确保右键选择“以管理员身份运行”启动此程序，或者使用“选择 Everything 导出的 CSV”功能进行查看。");
        }
        return Err(msg);
    }

    let run = (|| -> Result<(), String> {
        let (record_size_raw, total_records) = unsafe { get_volume_data_info(handle)? };
        let record_size = record_size_raw as usize;
        let mut cache: rustc_hash::FxHashMap<u64, TempMftEntry> = rustc_hash::FxHashMap::default();

        let mut probe = (total_records as i64) - 1;
        while probe >= 0 {
            let record_result = unsafe { read_record(handle, probe, record_size) };
            match record_result {
                Ok(Some((actual_record, record_bytes))) => {
                    let entry = match MftEntry::from_buffer(record_bytes, actual_record) {
                        Ok(entry) => entry,
                        Err(_) => {
                            probe = (actual_record as i64) - 1;
                            continue;
                        }
                    };

                    if entry.header.is_valid() && entry.is_allocated() {
                        // 仅保留主记录，过滤掉 Extension Records (非主记录)
                        if entry.header.base_reference.entry != 0 {
                            probe = (actual_record as i64) - 1;
                            continue;
                        }

                        let is_dir = entry.is_dir();
                        let name_attr = entry.find_best_name_attribute();

                        if let Some(attr) = name_attr {
                            let name = attr.name.clone();
                            let parent_id = attr.parent.entry;
                            
                            // 从 $DATA (0x80) 属性中获取文件的真实/最新大小，防止大小缩水
                            let mut size = 0u64;
                            if !is_dir {
                                let mut found_data = false;
                                for attr_res in entry.iter_attributes() {
                                    if let Ok(attribute) = attr_res {
                                        if matches!(attribute.header.type_code, mft::attribute::MftAttributeType::DATA) {
                                            if attribute.header.name.is_empty() {
                                                match &attribute.header.residential_header {
                                                    mft::attribute::header::ResidentialHeader::Resident(res_header) => {
                                                        size = res_header.data_size as u64;
                                                        found_data = true;
                                                        break;
                                                    }
                                                    mft::attribute::header::ResidentialHeader::NonResident(non_res_header) => {
                                                        size = non_res_header.file_size;
                                                        found_data = true;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if !found_data {
                                    size = attr.logical_size;
                                }
                            }

                            cache.insert(
                                actual_record,
                                TempMftEntry {
                                    parent_id,
                                    name,
                                    is_dir,
                                    size,
                                },
                            );
                        }
                    }
                    probe = (actual_record as i64) - 1;
                }
                Ok(None) => {
                    break;
                }
                Err(_) => {
                    // 读取失败（可能由于 total_records 估算过大导致越界）。
                    // 我们不采取耗时极长的单步 probe -= 1，而是使用二分查找在 O(log N) 步内瞬间定位到最大有效的 MFT record。
                    let mut low = 0;
                    let mut high = probe - 1;
                    let mut found_valid = None;
                    
                    while low <= high {
                        let mid = low + (high - low) / 2;
                        let test_res = unsafe { read_record(handle, mid, record_size) };
                        match test_res {
                            Ok(Some((actual_record, _))) => {
                                found_valid = Some(actual_record);
                                low = mid + 1;
                            }
                            Ok(None) => {
                                high = mid - 1;
                            }
                            Err(_) => {
                                high = mid - 1;
                            }
                        }
                    }
                    
                    if let Some(valid_record) = found_valid {
                        probe = valid_record as i64;
                    } else {
                        break;
                    }
                }
            }
        }

        let drive_letter = file_path.chars().next().unwrap_or('C');
        let drive_prefix = format!("{}:", drive_letter);

        // 1. 建立节点邻接表以支持 O(N) 的 DFS 高性能树构建
        let mut parent_to_children: rustc_hash::FxHashMap<u64, Vec<u64>> = rustc_hash::FxHashMap::default();
        for (&id, entry) in &cache {
            if id != 5 {
                parent_to_children.entry(entry.parent_id).or_default().push(id);
            }
        }

        // 2. 递归 DFS 构建节点树，每一项仅访问一次，杜绝字符串路径分割和哈希重复查找
        fn build_node_recursive(
            id: u64,
            current_path: String,
            cache: &rustc_hash::FxHashMap<u64, TempMftEntry>,
            parent_to_children: &rustc_hash::FxHashMap<u64, Vec<u64>>,
            visited: &mut rustc_hash::FxHashSet<u64>,
        ) -> FileNode {
            if !visited.insert(id) {
                return FileNode::new("[Circular]".to_string(), current_path, false);
            }
            let entry = &cache[&id];
            let mut node = FileNode::new(entry.name.clone(), current_path.clone(), entry.is_dir);
            node.size = if entry.is_dir { 0 } else { entry.size };

            if let Some(children_ids) = parent_to_children.get(&id) {
                for &child_id in children_ids {
                    if let Some(child_entry) = cache.get(&child_id) {
                        let child_path = if current_path.is_empty() {
                            child_entry.name.clone()
                        } else {
                            format!("{}\\{}", current_path, child_entry.name)
                        };
                        let child_node = build_node_recursive(
                            child_id,
                            child_path,
                            cache,
                            parent_to_children,
                            visited,
                        );
                        node.size += child_node.size;
                        node.children.insert(child_entry.name.clone(), child_node);
                    }
                }
            }
            node
        }

        let mut computer_root = FileNode::root();
        if cache.contains_key(&5) {
            let mut visited = rustc_hash::FxHashSet::default();
            let mut root_c = build_node_recursive(
                5,
                drive_prefix.clone(),
                &cache,
                &parent_to_children,
                &mut visited,
            );

            // 3. 收集在用但由于父目录缺失而无法回溯到 5 的有效孤立文件，统一挂载在 [Lost Files] 下
            let mut lost_files = Vec::new();
            for (&id, entry) in &cache {
                if !visited.contains(&id) && !entry.is_dir && entry.size > 0 {
                    lost_files.push((id, entry));
                }
            }

            if !lost_files.is_empty() {
                let lost_dir_name = "[Lost Files]".to_string();
                let lost_dir_path = format!("{}\\{}", drive_prefix, lost_dir_name);
                let mut lost_dir_node = FileNode::new(lost_dir_name.clone(), lost_dir_path, true);

                for (_id, entry) in lost_files {
                    let file_path = format!("{}\\{}", lost_dir_node.full_path, entry.name);
                    let mut file_node = FileNode::new(entry.name.clone(), file_path, false);
                    file_node.size = entry.size;
                    lost_dir_node.size += entry.size;
                    lost_dir_node.children.insert(entry.name.clone(), file_node);
                }

                root_c.size += lost_dir_node.size;
                root_c.children.insert(lost_dir_name, lost_dir_node);
            }

            // 更新 C 盘节点名称与路径
            root_c.name = drive_prefix.clone();
            root_c.full_path = drive_prefix.clone();

            computer_root.size = root_c.size;
            computer_root.children.insert(drive_prefix, root_c);
        }

        tx.send(Ok(computer_root)).map_err(|_| "加载已取消".to_string())?;
        Ok(())
    })();

    unsafe {
        CloseHandle(handle);
    }

    run
}

#[cfg(not(windows))]
fn open_mft_file(file_path: &str) -> Result<File, String> {
    File::open(file_path).map_err(|e| format!("打开 MFT 失败: {e}"))
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

#[allow(dead_code)]
fn split_windows_path(path: &str) -> Option<(String, String)> {
    let mut parts = path.rsplitn(2, '\\');
    let name = parts.next()?.to_string();
    let parent = parts.next().unwrap_or("").to_string();
    Some((parent, name))
}

fn normalize_windows_str_path(path: &str) -> String {
    path.split('\\')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}

#[allow(dead_code)]
fn normalize_mft_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::Prefix(prefix) => parts.push(prefix.as_os_str().to_string_lossy().into_owned()),
            _ => {}
        }
    }
    parts.join("\\")
}
