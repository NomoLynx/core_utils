use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};
use xml::{reader::XmlEvent, EventReader};

/// Converts a file path to a file URI format.
pub fn to_file_uri(path: &str) -> String {
    let p = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let p = p.to_string_lossy().replace("\\", "/");

    format!("file:///{}", p)
}

/// Extracts the file extension from a given file path, returning it as an Option<&str>.
pub fn get_file_extension(file_path: &str) -> Option<&str> {
    if let Some(extension) = Path::new(file_path).extension() {
        extension.to_str()
    } else {
        None
    }
}

/// Extracts the file name without its extension from a given file path, returning it as an Option<String>.
pub fn get_file_name_without_extension(file_path: &str) -> Option<String> {
    std::path::Path::new(file_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .map(|x| x.to_string())
}

pub fn get_file_containing_folder(file_path: &str) -> Option<String> {
    std::path::Path::new(file_path)
        .parent()
        .and_then(|parent| parent.to_str())
        .map(|s| s.to_string())
}

pub fn get_files_in_folder(folder_path: &str, extension_str: &str) -> Vec<String> {
    if !std::path::Path::new(folder_path).exists() {
        return Vec::default();
    }

    let mut file_paths = Vec::new();
    let Ok(entries) = fs::read_dir(folder_path) else {
        return Vec::default();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let extension_option = path.extension();
        if let Some(extension) = extension_option {
            let current_ext = extension.to_string_lossy();
            if path.is_file()
                && (current_ext == extension_str || &format!(".{current_ext}") == extension_str)
            {
                if let Some(path_string) = path.to_str() {
                    file_paths.push(path_string.to_owned());
                }
            }
        }
    }

    file_paths
}

pub fn file_exists<S>(file_path: S) -> bool
where
    S: AsRef<str>,
{
    if let Ok(metadata) = fs::metadata(file_path.as_ref()) {
        metadata.is_file()
    } else {
        false
    }
}

pub fn path_file_exists(folder: &str, file: &str) -> bool {
    let path_buff = PathBuf::new();
    let file_path = path_buff.join(folder).join(file);
    if let Some(path_str) = file_path.to_str() {
        file_exists(path_str)
    } else {
        false
    }
}

pub fn folder_exists(path: &str) -> bool {
    if let Ok(metadata) = fs::metadata(path) {
        metadata.is_dir()
    } else {
        false
    }
}

pub fn read_file_to_string<S>(path: S) -> String
where
    S: AsRef<str>,
{
    if std::fs::metadata(path.as_ref()).is_ok() {
        let path2 = Path::new(path.as_ref());

        if let Ok(s) = read_file_content(path2) {
            s
        } else {
            panic!("file at {:?} has error", path2)
        }
    } else {
        panic!("file at {} cannot be found", path.as_ref())
    }
}

pub fn read_file_content<P: AsRef<Path>>(file_path: P) -> std::io::Result<String> {
    let mut file = std::fs::File::open(file_path)?;
    let mut contents = String::new();
    std::io::Read::read_to_string(&mut file, &mut contents)?;
    Ok(contents)
}

pub fn read_lines_except_comment(filename: &str) -> Vec<String> {
    std::fs::read_to_string(filename)
        .unwrap()
        .lines()
        .map(String::from)
        .filter(|x| !x.starts_with("//"))
        .collect()
}

pub fn read_xml_string(file_path: &Path, target_node: &str) -> Option<String> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    let parser = EventReader::new(reader);

    let mut node_content = String::new();

    for event in parser {
        match event {
            Ok(XmlEvent::StartElement { name, .. }) if name.local_name == target_node => {
                node_content = String::new();
            }
            Ok(XmlEvent::Characters(content)) => {
                node_content.push_str(&content);
            }
            Ok(XmlEvent::EndElement { name }) if name.local_name == target_node => {
                return Some(node_content);
            }
            _ => {}
        }
    }

    None
}

pub fn write_to_file(file_name: &str, content: &str) -> std::io::Result<()> {
    let mut file = File::create(file_name)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

pub fn write_to_file_option(file_name_option: Option<&String>, content: &str) -> std::io::Result<()> {
    if let Some(file_name) = file_name_option {
        write_to_file(file_name, content)?;
    }
    Ok(())
}

pub fn append_to_file(file_name: &str, content: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(file_name)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

pub fn append_to_file_option(file_name_option: Option<&String>, content: &str) -> std::io::Result<()> {
    if let Some(file_name) = file_name_option {
        append_to_file(file_name, content)?;
    }
    Ok(())
}

pub fn delete_file_option(file_name_option: Option<&String>) -> std::io::Result<()> {
    if let Some(file_name) = file_name_option {
        if Path::new(file_name).exists() {
            fs::remove_file(file_name)?;
        }
    }
    Ok(())
}
