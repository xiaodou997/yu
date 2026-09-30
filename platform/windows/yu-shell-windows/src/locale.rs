//! Windows shell strings.
//!
//! The shell deliberately keeps only platform chrome text here. Markdown
//! semantics and editor-facing labels belong to shared Rust layers.
//!
//! Windows follows the same release-language set as the macOS product:
//! English, Simplified Chinese, Traditional Chinese, Japanese and Korean.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    English,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
}

impl Locale {
    #[must_use]
    pub fn from_language_tag(tag: &str) -> Self {
        let normalized = tag.replace('_', "-").to_ascii_lowercase();
        if normalized == "zh-cn" || normalized == "zh-sg" || normalized.starts_with("zh-hans") {
            Self::SimplifiedChinese
        } else if normalized == "zh-tw"
            || normalized == "zh-hk"
            || normalized == "zh-mo"
            || normalized.starts_with("zh-hant")
        {
            Self::TraditionalChinese
        } else if normalized == "ja" || normalized.starts_with("ja-") {
            Self::Japanese
        } else if normalized == "ko" || normalized.starts_with("ko-") {
            Self::Korean
        } else {
            Self::English
        }
    }

    #[must_use]
    pub const fn strings(self) -> Strings {
        Strings { locale: self }
    }

    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::English,
            Self::SimplifiedChinese,
            Self::TraditionalChinese,
            Self::Japanese,
            Self::Korean,
        ]
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
            Locale::SimplifiedChinese | Locale::TraditionalChinese => "未命名",
            Locale::Japanese => "名称未設定",
            Locale::Korean => "제목 없음",
        }
    }

    #[must_use]
    pub const fn file(self) -> &'static str {
        match self.locale {
            Locale::English => "&File",
            Locale::SimplifiedChinese => "文件(&F)",
            Locale::TraditionalChinese => "檔案(&F)",
            Locale::Japanese => "ファイル(&F)",
            Locale::Korean => "파일(&F)",
        }
    }

    #[must_use]
    pub const fn edit(self) -> &'static str {
        match self.locale {
            Locale::English => "&Edit",
            Locale::SimplifiedChinese => "编辑(&E)",
            Locale::TraditionalChinese => "編輯(&E)",
            Locale::Japanese => "編集(&E)",
            Locale::Korean => "편집(&E)",
        }
    }

    #[must_use]
    pub const fn view(self) -> &'static str {
        match self.locale {
            Locale::English => "&View",
            Locale::SimplifiedChinese => "显示(&V)",
            Locale::TraditionalChinese => "顯示(&V)",
            Locale::Japanese => "表示(&V)",
            Locale::Korean => "보기(&V)",
        }
    }

    #[must_use]
    pub const fn new_document(self) -> &'static str {
        match self.locale {
            Locale::English => "&New\tCtrl+N",
            Locale::SimplifiedChinese => "新建(&N)\tCtrl+N",
            Locale::TraditionalChinese => "新增(&N)\tCtrl+N",
            Locale::Japanese => "新規作成(&N)\tCtrl+N",
            Locale::Korean => "새로 만들기(&N)\tCtrl+N",
        }
    }

    #[must_use]
    pub const fn open(self) -> &'static str {
        match self.locale {
            Locale::English => "&Open…\tCtrl+O",
            Locale::SimplifiedChinese => "打开(&O)…\tCtrl+O",
            Locale::TraditionalChinese => "開啟(&O)…\tCtrl+O",
            Locale::Japanese => "開く(&O)…\tCtrl+O",
            Locale::Korean => "열기(&O)…\tCtrl+O",
        }
    }

    #[must_use]
    pub const fn save(self) -> &'static str {
        match self.locale {
            Locale::English => "&Save\tCtrl+S",
            Locale::SimplifiedChinese => "保存(&S)\tCtrl+S",
            Locale::TraditionalChinese => "儲存(&S)\tCtrl+S",
            Locale::Japanese => "保存(&S)\tCtrl+S",
            Locale::Korean => "저장(&S)\tCtrl+S",
        }
    }

    #[must_use]
    pub const fn save_as(self) -> &'static str {
        match self.locale {
            Locale::English => "Save &As…\tCtrl+Shift+S",
            Locale::SimplifiedChinese => "另存为(&A)…\tCtrl+Shift+S",
            Locale::TraditionalChinese => "另存新檔(&A)…\tCtrl+Shift+S",
            Locale::Japanese => "別名で保存(&A)…\tCtrl+Shift+S",
            Locale::Korean => "다른 이름으로 저장(&A)…\tCtrl+Shift+S",
        }
    }

    #[must_use]
    pub const fn exit(self) -> &'static str {
        match self.locale {
            Locale::English => "E&xit",
            Locale::SimplifiedChinese => "退出(&X)",
            Locale::TraditionalChinese => "結束(&X)",
            Locale::Japanese => "終了(&X)",
            Locale::Korean => "종료(&X)",
        }
    }

    #[must_use]
    pub const fn undo(self) -> &'static str {
        match self.locale {
            Locale::English => "&Undo\tCtrl+Z",
            Locale::SimplifiedChinese => "撤销(&U)\tCtrl+Z",
            Locale::TraditionalChinese => "還原(&U)\tCtrl+Z",
            Locale::Japanese => "取り消す(&U)\tCtrl+Z",
            Locale::Korean => "실행 취소(&U)\tCtrl+Z",
        }
    }

    #[must_use]
    pub const fn redo(self) -> &'static str {
        match self.locale {
            Locale::English => "&Redo\tCtrl+Y",
            Locale::SimplifiedChinese => "重做(&R)\tCtrl+Y",
            Locale::TraditionalChinese => "重做(&R)\tCtrl+Y",
            Locale::Japanese => "やり直す(&R)\tCtrl+Y",
            Locale::Korean => "다시 실행(&R)\tCtrl+Y",
        }
    }

    #[must_use]
    pub const fn toggle_sidebar(self) -> &'static str {
        match self.locale {
            Locale::English => "&Sidebar\tCtrl+Shift+L",
            Locale::SimplifiedChinese => "侧边栏(&S)\tCtrl+Shift+L",
            Locale::TraditionalChinese => "側邊欄(&S)\tCtrl+Shift+L",
            Locale::Japanese => "サイドバー(&S)\tCtrl+Shift+L",
            Locale::Korean => "사이드바(&S)\tCtrl+Shift+L",
        }
    }

    #[must_use]
    pub const fn files(self) -> &'static str {
        match self.locale {
            Locale::English => "Files",
            Locale::SimplifiedChinese => "文件",
            Locale::TraditionalChinese => "檔案",
            Locale::Japanese => "ファイル",
            Locale::Korean => "파일",
        }
    }

    #[must_use]
    pub const fn outline(self) -> &'static str {
        match self.locale {
            Locale::English => "Outline",
            Locale::SimplifiedChinese => "大纲",
            Locale::TraditionalChinese => "大綱",
            Locale::Japanese => "アウトライン",
            Locale::Korean => "개요",
        }
    }

    #[must_use]
    pub const fn search(self) -> &'static str {
        match self.locale {
            Locale::English => "Search",
            Locale::SimplifiedChinese => "搜索",
            Locale::TraditionalChinese => "搜尋",
            Locale::Japanese => "検索",
            Locale::Korean => "검색",
        }
    }

    #[must_use]
    pub const fn editor_surface_pending(self) -> &'static str {
        match self.locale {
            Locale::English => "Editor surface — renderer connects in Windows group 3",
            Locale::SimplifiedChinese => "编辑器表面 — Windows 第三组接入渲染器",
            Locale::TraditionalChinese => "編輯器表面 — Windows 第三組接入渲染器",
            Locale::Japanese => "エディター領域 — Windows 第3グループでレンダラーを接続",
            Locale::Korean => "편집기 영역 — Windows 3그룹에서 렌더러 연결",
        }
    }

    #[must_use]
    pub const fn ready(self) -> &'static str {
        match self.locale {
            Locale::English => "Ready",
            Locale::SimplifiedChinese => "就绪",
            Locale::TraditionalChinese => "就緒",
            Locale::Japanese => "準備完了",
            Locale::Korean => "준비됨",
        }
    }

    #[must_use]
    pub const fn save_changes_question(self) -> &'static str {
        match self.locale {
            Locale::English => "Save changes before closing?",
            Locale::SimplifiedChinese => "关闭前保存更改吗？",
            Locale::TraditionalChinese => "關閉前要儲存變更嗎？",
            Locale::Japanese => "閉じる前に変更を保存しますか？",
            Locale::Korean => "닫기 전에 변경 사항을 저장하시겠습니까?",
        }
    }

    #[must_use]
    pub const fn external_change_question(self) -> &'static str {
        match self.locale {
            Locale::English => "The file changed outside Yu. Discard local changes and close?",
            Locale::SimplifiedChinese => "文件已被外部修改。是否放弃本地更改并关闭？",
            Locale::TraditionalChinese => "檔案已在 Yu 外部變更。要放棄本機變更並關閉嗎？",
            Locale::Japanese => {
                "ファイルが Yu の外部で変更されました。ローカルの変更を破棄して閉じますか？"
            }
            Locale::Korean => {
                "파일이 Yu 외부에서 변경되었습니다. 로컬 변경 사항을 버리고 닫으시겠습니까?"
            }
        }
    }

    #[must_use]
    pub const fn error_title(self) -> &'static str {
        match self.locale {
            Locale::English => "Yu Error",
            Locale::SimplifiedChinese => "Yu 错误",
            Locale::TraditionalChinese => "Yu 錯誤",
            Locale::Japanese => "Yu エラー",
            Locale::Korean => "Yu 오류",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_language_tags_select_the_same_five_languages_as_macos() {
        for tag in ["zh-CN", "zh_Hans", "zh-Hans-CN", "zh-SG"] {
            assert_eq!(Locale::from_language_tag(tag), Locale::SimplifiedChinese);
        }
        for tag in ["zh-TW", "zh_Hant", "zh-HK"] {
            assert_eq!(Locale::from_language_tag(tag), Locale::TraditionalChinese);
        }
        assert_eq!(Locale::from_language_tag("ja-JP"), Locale::Japanese);
        assert_eq!(Locale::from_language_tag("ko-KR"), Locale::Korean);
        assert_eq!(Locale::from_language_tag("en-US"), Locale::English);
        assert_eq!(Locale::from_language_tag("fr-FR"), Locale::English);
    }

    #[test]
    fn every_release_locale_exposes_the_shell_commands() {
        for locale in Locale::all() {
            let strings = locale.strings();
            assert!(!strings.file().is_empty());
            assert!(strings.new_document().contains("Ctrl+N"));
            assert!(strings.open().contains("Ctrl+O"));
            assert!(strings.save().contains("Ctrl+S"));
            assert!(!strings.editor_surface_pending().is_empty());
            assert!(!strings.save_changes_question().is_empty());
        }
    }
}
