use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::fs::File;
use std::sync::mpsc::Sender;

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

pub fn build_tree_stream(file_path: String, tx: Sender<Result<FileNode, String>>) {
    std::thread::spawn(move || {
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(file);

        let mut local_root = FileNode::new("Computer".into(), "".into(), true);
        let mut count = 0;

        for result in rdr.deserialize::<EverythingRecord>() {
            if let Ok(record) = result {
                let item_size = record.size.unwrap_or(0);
                let is_dir = record.size.is_none();
                let parts: Vec<&str> = record.path.split('\\').filter(|s| !s.is_empty()).collect();
                let mut current_node = &mut local_root;
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
                        .or_insert_with(|| {
                            FileNode::new(part.to_string(), current_path.clone(), true)
                        });
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

                count += 1;
                if count % 2000 == 0 {
                    if tx.send(Ok(local_root)).is_err() {
                        return;
                    }
                    local_root = FileNode::new("Computer".into(), "".into(), true);
                }
            }
        }
        let _ = tx.send(Ok(local_root));
    });
}
