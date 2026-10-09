//! Bounded path-only navigation history. Production persistence uses one native
//! REG_MULTI_SZ value; tests exercise the codec without touching user settings.
use std::io;
use std::path::{Path, PathBuf};

pub const LIMIT: usize = 12;
const MAX_UNITS: usize = 32_768;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavigationHistory {
    pub files: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
    pub last_folder: Option<PathBuf>,
}

impl NavigationHistory {
    pub fn note_file(&mut self, path: &Path) {
        if path.is_file() {
            note(&mut self.files, path);
        }
    }
    pub fn note_folder(&mut self, path: &Path) {
        if path.is_dir() {
            note(&mut self.folders, path);
            self.last_folder = self.folders.first().cloned();
        }
    }
    pub fn clear_files(&mut self) {
        self.files.clear();
    }
    pub fn clear_folders(&mut self) {
        self.folders.clear();
        self.last_folder = None;
    }

    fn encode(&self) -> io::Result<Vec<u16>> {
        let mut result = "YuNavigation1\0".encode_utf16().collect::<Vec<_>>();
        let records = self
            .last_folder
            .iter()
            .map(|p| ('W', p))
            .chain(self.files.iter().take(LIMIT).map(|p| ('F', p)))
            .chain(self.folders.iter().take(LIMIT).map(|p| ('D', p)));
        for (kind, path) in records {
            let Some(path) = path.to_str() else {
                continue;
            };
            if !Path::new(path).is_absolute() || path.contains('\0') {
                continue;
            }
            let record = format!("{kind}{path}").encode_utf16().collect::<Vec<_>>();
            // A long path must not make recent metadata unbounded or block open.
            if result.len() + record.len() + 2 > MAX_UNITS {
                continue;
            }
            result.extend(record);
            result.push(0);
        }
        result.push(0);
        Ok(result)
    }

    fn decode(data: &[u16]) -> io::Result<Self> {
        let invalid = || io::Error::from(io::ErrorKind::InvalidData);
        if data.len() > MAX_UNITS || !data.ends_with(&[0, 0]) {
            return Err(invalid());
        }
        let text = String::from_utf16(data).map_err(|_| invalid())?;
        let mut records = text[..text.len() - 1].split_terminator('\0');
        if records.next() != Some("YuNavigation1") {
            return Err(invalid());
        }
        let mut history = Self::default();
        for record in records.take(LIMIT * 2 + 1) {
            let Some(kind) = record.chars().next() else {
                return Err(invalid());
            };
            if !matches!(kind, 'F' | 'D' | 'W') {
                return Err(invalid());
            }
            let path = PathBuf::from(&record[1..]);
            if !path.is_absolute() {
                return Err(invalid());
            }
            match kind {
                'F' if history.files.len() < LIMIT => push_unique(&mut history.files, path),
                'D' if history.folders.len() < LIMIT => push_unique(&mut history.folders, path),
                'W' if history.last_folder.is_none() => history.last_folder = Some(path),
                _ => {}
            }
        }
        Ok(history)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(target_os = "windows") {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}
fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|old| same_path(old, &path)) {
        paths.push(path);
    }
}
fn note(paths: &mut Vec<PathBuf>, path: &Path) {
    let Ok(path) = path.canonicalize() else {
        return;
    };
    paths.retain(|old| !same_path(old, &path));
    paths.insert(0, path);
    paths.truncate(LIMIT);
}

#[cfg(target_os = "windows")]
mod registry {
    use super::*;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::System::Registry::*;
    use windows::core::w;
    const KEY: windows::core::PCWSTR = w!("Software\\YuMarkdown\\Navigation");
    fn checked(status: WIN32_ERROR) -> io::Result<()> {
        if status.0 == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status.0 as i32))
        }
    }
    impl NavigationHistory {
        pub fn load() -> io::Result<Self> {
            let mut bytes = 0u32;
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    KEY,
                    w!("History"),
                    RRF_RT_REG_MULTI_SZ,
                    None,
                    None,
                    Some(&mut bytes),
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(Self::default());
            }
            checked(status)?;
            if bytes < 4 || bytes as usize > MAX_UNITS * 2 || bytes % 2 != 0 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let mut data = vec![0u16; bytes as usize / 2];
            checked(unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    KEY,
                    w!("History"),
                    RRF_RT_REG_MULTI_SZ,
                    None,
                    Some(data.as_mut_ptr().cast()),
                    Some(&mut bytes),
                )
            })?;
            if bytes as usize > data.len() * 2 || bytes % 2 != 0 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            data.truncate(bytes as usize / 2);
            Self::decode(&data)
        }
        pub fn save(&self) -> io::Result<()> {
            let data = self.encode()?;
            let mut key = HKEY::default();
            checked(unsafe { RegCreateKeyW(HKEY_CURRENT_USER, KEY, &mut key) })?;
            let bytes = data
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            let result = checked(unsafe {
                RegSetValueExW(key, w!("History"), 0, REG_MULTI_SZ, Some(&bytes))
            });
            let _ = unsafe { RegCloseKey(key) };
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_history_roundtrip_is_unicode_bounded_and_clearable() {
        let root = std::env::temp_dir().join(format!("yu-history-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("root");
        let mut history = NavigationHistory::default();
        history.note_folder(&root);
        for i in 0..20 {
            let file = root.join(format!("羽🙂-{i}.md"));
            std::fs::write(&file, "unchanged").expect("file");
            history.note_file(&file);
        }
        assert_eq!(history.files.len(), LIMIT);
        let first = history.files[0].clone();
        history.note_file(&first);
        assert_eq!(history.files.len(), LIMIT);
        let restored =
            NavigationHistory::decode(&history.encode().expect("encode")).expect("decode");
        assert_eq!(restored, history);
        history.clear_files();
        assert!(history.files.is_empty() && history.last_folder.is_some());
        assert_eq!(std::fs::read_to_string(&first).expect("file"), "unchanged");
        history.clear_folders();
        assert_eq!(
            NavigationHistory::decode(&history.encode().expect("encode")).expect("decode"),
            NavigationHistory::default()
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn corrupt_or_future_history_is_not_interpreted_as_paths() {
        for data in [
            vec![],
            vec![0, 0],
            vec![0xd800, 0, 0],
            "YuNavigation2\0\0".encode_utf16().collect(),
            "YuNavigation1\0Frelative\0\0".encode_utf16().collect(),
            vec![1; MAX_UNITS + 1],
        ] {
            assert!(NavigationHistory::decode(&data).is_err());
        }
    }
}
