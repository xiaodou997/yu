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
pub enum WorkspaceText {
    OpenFolder,
    QuickOpen,
    Refresh,
    DocumentFolder,
    Filter,
    Files,
    Scanning,
    NoMatches,
    Partial,
    Unavailable,
    FileUnavailable,
    Open,
}

#[derive(Clone, Copy, Debug)]
pub enum NavigationText {
    SearchOptions,
    MatchCase,
    WholeWords,
    RecentFiles,
    RecentFolders,
    ClearFiles,
    ClearFolders,
}

#[derive(Clone, Copy, Debug)]
pub struct Strings {
    locale: Locale,
}

impl Strings {
    pub fn navigation(self, text: NavigationText) -> &'static str {
        let values = match self.locale {
            Locale::English => [
                "Search Options",
                "Match Case",
                "Whole Words",
                "Open Recent",
                "Recent Folders",
                "Clear Recent Files",
                "Clear Recent Folders",
            ],
            Locale::SimplifiedChinese => [
                "查找选项",
                "区分大小写",
                "全词匹配",
                "最近打开的文件",
                "最近的文件夹",
                "清除最近文件",
                "清除最近文件夹",
            ],
            Locale::TraditionalChinese => [
                "尋找選項",
                "區分大小寫",
                "全字匹配",
                "最近開啟的檔案",
                "最近的資料夾",
                "清除最近檔案",
                "清除最近資料夾",
            ],
            Locale::Japanese => [
                "検索オプション",
                "大文字と小文字を区別",
                "単語単位",
                "最近開いたファイル",
                "最近のフォルダ",
                "最近のファイルを消去",
                "最近のフォルダを消去",
            ],
            Locale::Korean => [
                "검색 옵션",
                "대소문자 구분",
                "단어 단위",
                "최근 파일",
                "최근 폴더",
                "최근 파일 지우기",
                "최근 폴더 지우기",
            ],
        };
        values[text as usize]
    }

    pub fn workspace(self, text: WorkspaceText) -> &'static str {
        let values = match self.locale {
            Locale::English => [
                "Open Folder…",
                "Quick Open…",
                "Refresh Files",
                "Use Document Folder",
                "Search file names or paths",
                "Workspace Files",
                "Scanning folder…",
                "No matching files",
                "Partial index; choose a smaller folder",
                "Folder unavailable. Choose another folder or refresh.",
                "File unavailable. Refresh the file list.",
                "Open",
            ],
            Locale::SimplifiedChinese => [
                "打开文件夹…",
                "快速打开…",
                "刷新文件",
                "使用文档所在文件夹",
                "搜索文件名或路径",
                "工作区文件",
                "正在扫描文件夹…",
                "没有匹配的文件",
                "索引不完整；请选择更小的文件夹",
                "文件夹不可用，请重新选择或刷新。",
                "文件不可用，请刷新文件列表。",
                "打开",
            ],
            Locale::TraditionalChinese => [
                "開啟資料夾…",
                "快速開啟…",
                "重新整理檔案",
                "使用文件所在資料夾",
                "搜尋檔名或路徑",
                "工作區檔案",
                "正在掃描資料夾…",
                "沒有符合的檔案",
                "索引不完整；請選擇較小的資料夾",
                "資料夾無法使用，請重新選擇或整理。",
                "檔案無法使用，請重新整理清單。",
                "開啟",
            ],
            Locale::Japanese => [
                "フォルダを開く…",
                "クイックオープン…",
                "ファイルを更新",
                "文書のフォルダを使用",
                "ファイル名またはパスを検索",
                "ワークスペースのファイル",
                "フォルダを検索中…",
                "一致するファイルがありません",
                "一部のみ索引済み：小さいフォルダを選択",
                "フォルダを利用できません。再選択または更新してください。",
                "ファイルを利用できません。一覧を更新してください。",
                "開く",
            ],
            Locale::Korean => [
                "폴더 열기…",
                "빠른 열기…",
                "파일 새로 고침",
                "문서 폴더 사용",
                "파일 이름 또는 경로 검색",
                "작업 공간 파일",
                "폴더 검색 중…",
                "일치하는 파일 없음",
                "일부만 색인됨: 더 작은 폴더를 선택하세요",
                "폴더를 사용할 수 없습니다. 다시 선택하거나 새로 고치세요.",
                "파일을 사용할 수 없습니다. 목록을 새로 고치세요.",
                "열기",
            ],
        };
        values[text as usize]
    }

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
    pub const fn help(self) -> &'static str {
        match self.locale {
            Locale::English => "&Help",
            Locale::SimplifiedChinese => "帮助(&H)",
            Locale::TraditionalChinese => "說明(&H)",
            Locale::Japanese => "ヘルプ(&H)",
            Locale::Korean => "도움말(&H)",
        }
    }

    #[must_use]
    pub const fn github_repository(self) -> &'static str {
        match self.locale {
            Locale::English => "GitHub Repository",
            Locale::SimplifiedChinese => "GitHub 开源项目",
            Locale::TraditionalChinese => "GitHub 開源專案",
            Locale::Japanese => "GitHub リポジトリ",
            Locale::Korean => "GitHub 저장소",
        }
    }

    #[must_use]
    pub const fn report_issue(self) -> &'static str {
        match self.locale {
            Locale::English => "Report an Issue…",
            Locale::SimplifiedChinese => "反馈问题…",
            Locale::TraditionalChinese => "回報問題…",
            Locale::Japanese => "問題を報告…",
            Locale::Korean => "문제 신고…",
        }
    }

    #[must_use]
    pub const fn about_yu(self) -> &'static str {
        match self.locale {
            Locale::English => "About Yu",
            Locale::SimplifiedChinese => "关于 Yu",
            Locale::TraditionalChinese => "關於 Yu",
            Locale::Japanese => "Yu について",
            Locale::Korean => "Yu 정보",
        }
    }

    #[must_use]
    pub const fn version(self) -> &'static str {
        match self.locale {
            Locale::English => "Version",
            Locale::SimplifiedChinese | Locale::TraditionalChinese => "版本",
            Locale::Japanese => "バージョン",
            Locale::Korean => "버전",
        }
    }

    #[must_use]
    pub const fn about_body(self) -> &'static str {
        match self.locale {
            Locale::English => {
                "Visual Markdown Editor\n\nYu is open source software. Use Help → GitHub Repository to view the source code. If you run into a problem or have a feature suggestion, use Help → Report an Issue… to open an issue on GitHub.\n\nLicense: Apache-2.0"
            }
            Locale::SimplifiedChinese => {
                "所见即所得 Markdown 编辑器\n\nYu 是开源软件。可通过“帮助 → GitHub 开源项目”查看源码；如果遇到问题或有功能建议，请使用“帮助 → 反馈问题…”前往 GitHub 提交 Issue。\n\n许可证：Apache-2.0"
            }
            Locale::TraditionalChinese => {
                "所見即所得 Markdown 編輯器\n\nYu 是開源軟體。可透過「說明 → GitHub 開源專案」查看原始碼；如果遇到問題或有功能建議，請使用「說明 → 回報問題…」前往 GitHub 提交 Issue。\n\n授權條款：Apache-2.0"
            }
            Locale::Japanese => {
                "WYSIWYG Markdown エディター\n\nYu はオープンソースソフトウェアです。「ヘルプ → GitHub リポジトリ」からソースコードを確認できます。問題や機能提案がある場合は、「ヘルプ → 問題を報告…」から GitHub Issue を作成してください。\n\nライセンス: Apache-2.0"
            }
            Locale::Korean => {
                "WYSIWYG Markdown 편집기\n\nYu는 오픈 소스 소프트웨어입니다. “도움말 → GitHub 저장소”에서 소스 코드를 볼 수 있습니다. 문제나 기능 제안이 있으면 “도움말 → 문제 신고…”를 사용해 GitHub Issue를 등록해 주세요.\n\n라이선스: Apache-2.0"
            }
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
    pub const fn replace(self) -> &'static str {
        match self.locale {
            Locale::English => "Replace",
            Locale::SimplifiedChinese => "替换",
            Locale::TraditionalChinese => "取代",
            Locale::Japanese => "置換",
            Locale::Korean => "바꾸기",
        }
    }

    #[must_use]
    pub const fn replace_all(self) -> &'static str {
        match self.locale {
            Locale::English => "Replace All",
            Locale::SimplifiedChinese => "全部替换",
            Locale::TraditionalChinese => "全部取代",
            Locale::Japanese => "すべて置換",
            Locale::Korean => "모두 바꾸기",
        }
    }

    #[must_use]
    pub const fn replace_with(self) -> &'static str {
        match self.locale {
            Locale::English => "Replace with",
            Locale::SimplifiedChinese => "替换为",
            Locale::TraditionalChinese => "取代為",
            Locale::Japanese => "置換後の文字列",
            Locale::Korean => "바꿀 내용",
        }
    }

    #[must_use]
    pub const fn find_and_replace(self) -> &'static str {
        match self.locale {
            Locale::English => "Find and Replace",
            Locale::SimplifiedChinese => "查找与替换",
            Locale::TraditionalChinese => "尋找與取代",
            Locale::Japanese => "検索と置換",
            Locale::Korean => "찾기 및 바꾸기",
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
    pub fn character_count(self, count: usize) -> String {
        match self.locale {
            Locale::English if count == 1 => "1 character".to_owned(),
            Locale::English => format!("{count} characters"),
            Locale::SimplifiedChinese => format!("{count} 字符"),
            Locale::TraditionalChinese => format!("{count} 字元"),
            Locale::Japanese => format!("{count}文字"),
            Locale::Korean => format!("{count}자"),
        }
    }

    #[must_use]
    pub const fn image_properties(self) -> &'static str {
        match self.locale {
            Locale::English => "Image Properties",
            Locale::SimplifiedChinese => "图片属性",
            Locale::TraditionalChinese => "圖片屬性",
            Locale::Japanese => "画像のプロパティ",
            Locale::Korean => "이미지 속성",
        }
    }

    #[must_use]
    pub const fn image_source(self) -> &'static str {
        match self.locale {
            Locale::English => "Image Source",
            Locale::SimplifiedChinese => "图片源码",
            Locale::TraditionalChinese => "圖片原始碼",
            Locale::Japanese => "画像ソース",
            Locale::Korean => "이미지 소스",
        }
    }

    #[must_use]
    pub const fn replace_image(self) -> &'static str {
        match self.locale {
            Locale::English => "Replace Image…",
            Locale::SimplifiedChinese => "替换图片…",
            Locale::TraditionalChinese => "替換圖片…",
            Locale::Japanese => "画像を置き換え…",
            Locale::Korean => "이미지 교체…",
        }
    }

    #[must_use]
    pub const fn image_size(self) -> &'static str {
        match self.locale {
            Locale::English => "Image Size",
            Locale::SimplifiedChinese => "图片大小",
            Locale::TraditionalChinese => "圖片大小",
            Locale::Japanese => "画像サイズ",
            Locale::Korean => "이미지 크기",
        }
    }

    #[must_use]
    pub const fn more_image_actions(self) -> &'static str {
        match self.locale {
            Locale::English => "More Image Actions",
            Locale::SimplifiedChinese => "更多图片操作",
            Locale::TraditionalChinese => "更多圖片操作",
            Locale::Japanese => "その他の画像操作",
            Locale::Korean => "추가 이미지 작업",
        }
    }

    #[must_use]
    pub const fn original_size(self) -> &'static str {
        match self.locale {
            Locale::English => "Original Size",
            Locale::SimplifiedChinese => "原始大小",
            Locale::TraditionalChinese => "原始大小",
            Locale::Japanese => "元のサイズ",
            Locale::Korean => "원본 크기",
        }
    }

    #[must_use]
    pub const fn fit_to_column(self) -> &'static str {
        match self.locale {
            Locale::English => "Fit to Column",
            Locale::SimplifiedChinese => "适应正文宽度",
            Locale::TraditionalChinese => "適應正文寬度",
            Locale::Japanese => "本文幅に合わせる",
            Locale::Korean => "본문 너비에 맞춤",
        }
    }

    #[must_use]
    pub const fn open_image(self) -> &'static str {
        match self.locale {
            Locale::English => "Open Image",
            Locale::SimplifiedChinese => "打开图片",
            Locale::TraditionalChinese => "開啟圖片",
            Locale::Japanese => "画像を開く",
            Locale::Korean => "이미지 열기",
        }
    }

    #[must_use]
    pub const fn show_in_explorer(self) -> &'static str {
        match self.locale {
            Locale::English => "Show in File Explorer",
            Locale::SimplifiedChinese => "在文件资源管理器中显示",
            Locale::TraditionalChinese => "在檔案總管中顯示",
            Locale::Japanese => "エクスプローラーで表示",
            Locale::Korean => "파일 탐색기에서 보기",
        }
    }

    #[must_use]
    pub const fn copy_code(self) -> &'static str {
        match self.locale {
            Locale::English => "Copy",
            Locale::SimplifiedChinese => "复制",
            Locale::TraditionalChinese => "複製",
            Locale::Japanese => "コピー",
            Locale::Korean => "복사",
        }
    }

    #[must_use]
    pub const fn copied_code(self) -> &'static str {
        match self.locale {
            Locale::English => "Copied",
            Locale::SimplifiedChinese => "已复制",
            Locale::TraditionalChinese => "已複製",
            Locale::Japanese => "完了",
            Locale::Korean => "완료",
        }
    }

    #[must_use]
    pub const fn copy_image(self) -> &'static str {
        match self.locale {
            Locale::English => "Copy Image",
            Locale::SimplifiedChinese => "复制图片",
            Locale::TraditionalChinese => "複製圖片",
            Locale::Japanese => "画像をコピー",
            Locale::Korean => "이미지 복사",
        }
    }

    #[must_use]
    pub const fn copy_image_address(self) -> &'static str {
        match self.locale {
            Locale::English => "Copy Image Address",
            Locale::SimplifiedChinese => "复制图片地址",
            Locale::TraditionalChinese => "複製圖片位址",
            Locale::Japanese => "画像アドレスをコピー",
            Locale::Korean => "이미지 주소 복사",
        }
    }

    #[must_use]
    pub const fn edit_markdown_source(self) -> &'static str {
        match self.locale {
            Locale::English => "Edit Markdown Source",
            Locale::SimplifiedChinese => "编辑 Markdown 源码",
            Locale::TraditionalChinese => "編輯 Markdown 原始碼",
            Locale::Japanese => "Markdownソースを編集",
            Locale::Korean => "Markdown 소스 편집",
        }
    }

    #[must_use]
    pub const fn image_address(self) -> &'static str {
        match self.locale {
            Locale::English => "Image Address",
            Locale::SimplifiedChinese => "图片地址",
            Locale::TraditionalChinese => "圖片位址",
            Locale::Japanese => "画像のアドレス",
            Locale::Korean => "이미지 주소",
        }
    }

    #[must_use]
    pub const fn alternative_text(self) -> &'static str {
        match self.locale {
            Locale::English => "Alternative Text",
            Locale::SimplifiedChinese => "替代文字",
            Locale::TraditionalChinese => "替代文字",
            Locale::Japanese => "代替テキスト",
            Locale::Korean => "대체 텍스트",
        }
    }

    #[must_use]
    pub const fn width(self) -> &'static str {
        match self.locale {
            Locale::English => "Width",
            Locale::SimplifiedChinese => "宽度",
            Locale::TraditionalChinese => "寬度",
            Locale::Japanese => "幅",
            Locale::Korean => "너비",
        }
    }

    #[must_use]
    pub const fn height(self) -> &'static str {
        match self.locale {
            Locale::English => "Height",
            Locale::SimplifiedChinese => "高度",
            Locale::TraditionalChinese => "高度",
            Locale::Japanese => "高さ",
            Locale::Korean => "높이",
        }
    }

    #[must_use]
    pub const fn lock_aspect_ratio(self) -> &'static str {
        match self.locale {
            Locale::English => "Lock Aspect Ratio",
            Locale::SimplifiedChinese => "锁定纵横比",
            Locale::TraditionalChinese => "鎖定長寬比",
            Locale::Japanese => "アスペクト比を固定",
            Locale::Korean => "가로세로 비율 고정",
        }
    }

    #[must_use]
    pub const fn apply(self) -> &'static str {
        match self.locale {
            Locale::English => "Apply",
            Locale::SimplifiedChinese => "应用",
            Locale::TraditionalChinese => "套用",
            Locale::Japanese => "適用",
            Locale::Korean => "적용",
        }
    }

    #[must_use]
    pub const fn cancel(self) -> &'static str {
        match self.locale {
            Locale::English => "Cancel",
            Locale::SimplifiedChinese | Locale::TraditionalChinese => "取消",
            Locale::Japanese => "キャンセル",
            Locale::Korean => "취소",
        }
    }

    #[must_use]
    pub const fn invalid_image_properties(self) -> &'static str {
        match self.locale {
            Locale::English => {
                "Enter an image address. Width and height must be whole numbers from 1 to 100000, or left blank."
            }
            Locale::SimplifiedChinese => "请填写图片地址；宽度和高度须为 1–100000 的整数或留空。",
            Locale::TraditionalChinese => "請填寫圖片位址；寬度與高度須為 1–100000 的整數或留白。",
            Locale::Japanese => {
                "画像のアドレスを入力してください。幅と高さは 1～100000 の整数にするか、空欄にしてください。"
            }
            Locale::Korean => {
                "이미지 주소를 입력하십시오. 너비와 높이는 1–100000 사이의 정수이거나 비워 두어야 합니다."
            }
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
    fn character_count_matches_release_locale_wording() {
        assert_eq!(Locale::English.strings().character_count(1), "1 character");
        assert_eq!(Locale::English.strings().character_count(2), "2 characters");
        assert_eq!(
            Locale::SimplifiedChinese.strings().character_count(26),
            "26 字符"
        );
        assert_eq!(
            Locale::TraditionalChinese.strings().character_count(26),
            "26 字元"
        );
        assert_eq!(Locale::Japanese.strings().character_count(26), "26文字");
        assert_eq!(Locale::Korean.strings().character_count(26), "26자");
    }

    #[test]
    fn every_release_locale_exposes_the_shell_commands() {
        for locale in Locale::all() {
            let strings = locale.strings();
            assert!(!strings.file().is_empty());
            assert!(!strings.help().is_empty());
            assert!(!strings.github_repository().is_empty());
            assert!(!strings.report_issue().is_empty());
            assert!(!strings.about_yu().is_empty());
            assert!(!strings.about_body().is_empty());
            assert!(!strings.character_count(0).is_empty());
            assert!(!strings.character_count(1).is_empty());
            assert!(strings.new_document().contains("Ctrl+N"));
            assert!(strings.open().contains("Ctrl+O"));
            assert!(strings.save().contains("Ctrl+S"));
            assert!(!strings.editor_surface_pending().is_empty());
            assert!(!strings.save_changes_question().is_empty());
            assert!(!strings.image_properties().is_empty());
            assert!(!strings.replace_image().is_empty());
            assert!(!strings.image_size().is_empty());
            assert!(!strings.edit_markdown_source().is_empty());
        }
    }
}
