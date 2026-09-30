//! Windows shell strings.
//!
//! The shell deliberately keeps only platform chrome text here. Markdown
//! semantics and editor-facing labels belong to shared Rust layers.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    English,
    SimplifiedChinese,
}

impl Locale {
    #[must_use]
    pub fn from_language_tag(tag: &str) -> Self {
        let normalized = tag.replace('_', "-").to_ascii_lowercase();
        if normalized == "zh-cn" || normalized == "zh-sg" || normalized.starts_with("zh-hans") {
            Self::SimplifiedChinese
        } else {
            Self::English
        }
    }

    #[must_use]
    pub const fn strings(self) -> Strings {
        Strings { locale: self }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Strings {
    locale: Locale,
}

impl Strings {
    #[must_use]
    pub const fn app_name(self) -> &'static str {
        "Yu"
    }

    #[must_use]
    pub const fn untitled(self) -> &'static str {
        match self.locale {
            Locale::English => "Untitled",
            Locale::SimplifiedChinese => "未命名",
        }
    }

    #[must_use]
    pub const fn file(self) -> &'static str {
        match self.locale {
            Locale::English => "&File",
            Locale::SimplifiedChinese => "文件(&F)",
        }
    }

    #[must_use]
    pub const fn edit(self) -> &'static str {
        match self.locale {
            Locale::English => "&Edit",
            Locale::SimplifiedChinese => "编辑(&E)",
        }
    }

    #[must_use]
    pub const fn view(self) -> &'static str {
        match self.locale {
            Locale::English => "&View",
            Locale::SimplifiedChinese => "查看(&V)",
        }
    }

    #[must_use]
    pub const fn new_document(self) -> &'static str {
        match self.locale {
            Locale::English => "&New\tCtrl+N",
            Locale::SimplifiedChinese => "新建(&N)\tCtrl+N",
        }
    }

    #[must_use]
    pub const fn open(self) -> &'static str {
        match self.locale {
            Locale::English => "&Open…\tCtrl+O",
            Locale::SimplifiedChinese => "打开(&O)…\tCtrl+O",
        }
    }

    #[must_use]
    pub const fn save(self) -> &'static str {
        match self.locale {
            Locale::English => "&Save\tCtrl+S",
            Locale::SimplifiedChinese => "保存(&S)\tCtrl+S",
        }
    }

    #[must_use]
    pub const fn save_as(self) -> &'static str {
        match self.locale {
            Locale::English => "Save &As…\tCtrl+Shift+S",
            Locale::SimplifiedChinese => "另存为(&A)…\tCtrl+Shift+S",
        }
    }

    #[must_use]
    pub const fn exit(self) -> &'static str {
        match self.locale {
            Locale::English => "E&xit",
            Locale::SimplifiedChinese => "退出(&X)",
        }
    }

    #[must_use]
    pub const fn undo(self) -> &'static str {
        match self.locale {
            Locale::English => "&Undo\tCtrl+Z",
            Locale::SimplifiedChinese => "撤销(&U)\tCtrl+Z",
        }
    }

    #[must_use]
    pub const fn redo(self) -> &'static str {
        match self.locale {
            Locale::English => "&Redo\tCtrl+Y",
            Locale::SimplifiedChinese => "重做(&R)\tCtrl+Y",
        }
    }

    #[must_use]
    pub const fn toggle_sidebar(self) -> &'static str {
        match self.locale {
            Locale::English => "&Sidebar\tCtrl+Shift+L",
            Locale::SimplifiedChinese => "侧边栏(&S)\tCtrl+Shift+L",
        }
    }

    #[must_use]
    pub const fn files(self) -> &'static str {
        match self.locale {
            Locale::English => "Files",
            Locale::SimplifiedChinese => "文件",
        }
    }

    #[must_use]
    pub const fn outline(self) -> &'static str {
        match self.locale {
            Locale::English => "Outline",
            Locale::SimplifiedChinese => "大纲",
        }
    }

    #[must_use]
    pub const fn search(self) -> &'static str {
        match self.locale {
            Locale::English => "Search",
            Locale::SimplifiedChinese => "搜索",
        }
    }

    #[must_use]
    pub const fn editor_surface_pending(self) -> &'static str {
        match self.locale {
            Locale::English => "Editor surface — renderer connects in Windows group 3",
            Locale::SimplifiedChinese => "编辑器表面 — Windows 第三组接入渲染器",
        }
    }

    #[must_use]
    pub const fn ready(self) -> &'static str {
        match self.locale {
            Locale::English => "Ready",
            Locale::SimplifiedChinese => "就绪",
        }
    }

    #[must_use]
    pub const fn save_changes_question(self) -> &'static str {
        match self.locale {
            Locale::English => "Save changes before closing?",
            Locale::SimplifiedChinese => "关闭前保存更改吗？",
        }
    }

    #[must_use]
    pub const fn external_change_question(self) -> &'static str {
        match self.locale {
            Locale::English => "The file changed outside Yu. Discard local changes and close?",
            Locale::SimplifiedChinese => "文件已被外部修改。是否放弃本地更改并关闭？",
        }
    }

    #[must_use]
    pub const fn error_title(self) -> &'static str {
        match self.locale {
            Locale::English => "Yu Error",
            Locale::SimplifiedChinese => "Yu 错误",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_chinese_tags_are_selected_explicitly() {
        for tag in ["zh-CN", "zh_Hans", "zh-Hans-CN", "zh-SG"] {
            assert_eq!(Locale::from_language_tag(tag), Locale::SimplifiedChinese);
        }
        assert_eq!(Locale::from_language_tag("zh-TW"), Locale::English);
        assert_eq!(Locale::from_language_tag("en-US"), Locale::English);
    }

    #[test]
    fn both_locales_expose_the_shell_commands() {
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            let strings = locale.strings();
            assert!(!strings.file().is_empty());
            assert!(strings.new_document().contains("Ctrl+N"));
            assert!(strings.open().contains("Ctrl+O"));
            assert!(strings.save().contains("Ctrl+S"));
            assert!(!strings.editor_surface_pending().is_empty());
        }
    }
}
