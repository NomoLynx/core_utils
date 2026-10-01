use std::fmt::Display;

#[derive(Clone)]
pub struct FileObject {
    file_name: String,
    file_extension : String,
    content: String,
}

impl FileObject {
    pub fn new(file_name: &str, file_extension: &str, content: &str) -> Self {
        Self {
            file_name: file_name.to_string(),
            file_extension: file_extension.to_string(),
            content: content.to_string(),
        }
    }

    pub fn get_file_name(&self) -> &str {
        &self.file_name
    }

    pub fn get_file_extension(&self) -> &str {
        &self.file_extension
    }

    pub fn get_content(&self) -> &str {
        &self.content
    }
}

impl Display for FileObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut r = String::new();
        r.push_str(format!("filename = {}.{}\r\n", self.file_name, self.file_extension).as_str());
        r.push_str(format!("{}\r\n", self.content).as_str());
        write!(f, "{}", r)
    }
}
