//! 桌面文件按扩展名分类(纯函数,无系统调用,单测友好)。
//!
//! 8 类扩展名分类 + 文件夹一类(目录不看扩展名),共 9 个分组:
//!   文件夹 / 文档 / 图片 / 视频 / 音频 / 压缩 / 程序 / 代码 / 其他
//! 规则表集中在 `classify_ext`,新增类别改一处即可,前端按 category 字符串
//! 显示中文名与图标(emoji 在前端映射)。

/// 类别显示名(即 DrawerGroup.category 字段,前端直接展示)
pub const CATEGORY_FOLDER: &str = "文件夹";
pub const CATEGORY_DOC: &str = "文档";
pub const CATEGORY_IMAGE: &str = "图片";
pub const CATEGORY_VIDEO: &str = "视频";
pub const CATEGORY_AUDIO: &str = "音频";
pub const CATEGORY_ARCHIVE: &str = "压缩";
pub const CATEGORY_PROGRAM: &str = "程序";
pub const CATEGORY_CODE: &str = "代码";
pub const CATEGORY_OTHER: &str = "其他";

/// 分组固定展示顺序(空组也返回,前端左侧分类 tab 稳定)
pub const CATEGORY_ORDER: [&str; 9] = [
    CATEGORY_FOLDER,
    CATEGORY_DOC,
    CATEGORY_IMAGE,
    CATEGORY_VIDEO,
    CATEGORY_AUDIO,
    CATEGORY_ARCHIVE,
    CATEGORY_PROGRAM,
    CATEGORY_CODE,
    CATEGORY_OTHER,
];

/// 文档类扩展名
const DOC_EXTS: &[&str] = &[
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pdf", "txt", "md", "csv", "rtf", "odt",
    "ods", "odp", "wps", "et", "dps", "one", "pages", "numbers", "key", "tex",
];

/// 图片类扩展名
const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "ico", "tif", "tiff", "heic",
    "heif", "avif", "raw", "psd",
];

/// 视频类扩展名
/// 注意:不含 "ts"——.ts 同时是 TypeScript 源码与 mpeg-ts 传输流,
/// 桌面场景 TS 源码远常见,归类代码(见 CODE_EXTS)。
const VIDEO_EXTS: &[&str] = &[
    "mp4", "avi", "mkv", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "rmvb",
    "rm", "3gp", "vob", "m2ts",
];

/// 音频类扩展名
const AUDIO_EXTS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "ape", "mid", "midi", "opus",
    "amr", "aiff", "dsd",
];

/// 压缩包类扩展名(tar.gz 由 gz 兜住)
const ARCHIVE_EXTS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "iso", "cab", "lz4", "lzh",
    "arj",
];

/// 程序/可执行类扩展名(含 lnk:快捷方式指向应用,归程序类而非"其他",
/// 抽屉/Dock 场景下大量桌面快捷方式据此归类)
const PROGRAM_EXTS: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "appx", "msix", "apk", "msixbundle",
    "appxbundle", "lnk",
];

/// 代码/配置类扩展名
const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "vue", "c", "h", "cpp", "hpp", "cc", "java",
    "cs", "go", "rb", "php", "html", "css", "json", "xml", "yaml", "yml", "toml", "sql",
    "sh", "ps1", "kt", "swift", "lua", "asm", "ino", "gradle", "ini", "cfg", "conf",
];

fn ext_in(ext: &str, table: &[&str]) -> bool {
    table.contains(&ext)
}

/// 按扩展名分类(不含点,大小写不敏感),返回类别显示名
pub fn classify_ext(ext: &str) -> &'static str {
    let e = ext.trim().to_ascii_lowercase();
    if e.is_empty() {
        return CATEGORY_OTHER;
    }
    if ext_in(&e, DOC_EXTS) {
        CATEGORY_DOC
    } else if ext_in(&e, IMAGE_EXTS) {
        CATEGORY_IMAGE
    } else if ext_in(&e, VIDEO_EXTS) {
        CATEGORY_VIDEO
    } else if ext_in(&e, AUDIO_EXTS) {
        CATEGORY_AUDIO
    } else if ext_in(&e, ARCHIVE_EXTS) {
        CATEGORY_ARCHIVE
    } else if ext_in(&e, PROGRAM_EXTS) {
        CATEGORY_PROGRAM
    } else if ext_in(&e, CODE_EXTS) {
        CATEGORY_CODE
    } else {
        CATEGORY_OTHER
    }
}

/// 按文件名与是否目录分类。
/// 目录 → 文件夹;普通文件取最后一个点号后的扩展名分类
/// (无扩展名/隐藏文件如 .gitignore 落入其他)。
pub fn classify_file(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return CATEGORY_FOLDER;
    }
    let ext = match name.rsplit_once('.') {
        Some((_, e)) => e,
        None => "",
    };
    classify_ext(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断言某文件名归入某类(辅助函数,减少重复)
    fn assert_class(name: &str, expect: &str) {
        assert_eq!(classify_file(name, false), expect, "文件 {name} 分类错误");
    }

    #[test]
    fn document_exts() {
        for n in ["报告.docx", "a.pdf", "b.txt", "c.md", "d.csv", "e.xlsx", "f.pptx", "g.rtf"] {
            assert_class(n, CATEGORY_DOC);
        }
    }

    #[test]
    fn image_exts() {
        for n in ["a.jpg", "b.jpeg", "c.png", "d.gif", "e.bmp", "f.webp", "g.svg", "h.ico"] {
            assert_class(n, CATEGORY_IMAGE);
        }
    }

    #[test]
    fn video_exts() {
        for n in ["a.mp4", "b.avi", "c.mkv", "d.mov", "e.wmv", "f.flv", "g.webm", "h.mpeg"] {
            assert_class(n, CATEGORY_VIDEO);
        }
    }

    #[test]
    fn audio_exts() {
        for n in ["a.mp3", "b.wav", "c.flac", "d.aac", "e.ogg", "f.m4a", "g.wma"] {
            assert_class(n, CATEGORY_AUDIO);
        }
    }

    #[test]
    fn archive_exts() {
        for n in ["a.zip", "b.rar", "c.7z", "d.tar", "e.tar.gz", "f.iso", "g.bz2"] {
            assert_class(n, CATEGORY_ARCHIVE);
        }
        // 多层扩展名:取最后一个点后的扩展名即可命中
        assert_class("backup.tar.gz", CATEGORY_ARCHIVE);
    }

    #[test]
    fn program_exts() {
        for n in ["a.exe", "b.msi", "c.bat", "d.cmd", "e.appx", "f.apk"] {
            assert_class(n, CATEGORY_PROGRAM);
        }
    }

    #[test]
    fn lnk_goes_to_program() {
        // 桌面快捷方式(联想浏览器.lnk 等)归程序类,不落"其他"
        assert_class("联想浏览器.lnk", CATEGORY_PROGRAM);
        assert_class("a.LNK", CATEGORY_PROGRAM);
    }

    #[test]
    fn code_exts() {
        for n in ["a.rs", "b.py", "c.ts", "d.vue", "e.cpp", "f.java", "g.sh", "h.json", "i.yaml", "j.toml"] {
            assert_class(n, CATEGORY_CODE);
        }
    }

    #[test]
    fn uppercase_ext_is_case_insensitive() {
        for n in ["A.PDF", "B.JPG", "C.ZIP", "D.EXE", "E.RS", "F.MP4"] {
            assert_class(n, classify_ext(&n.rsplit_once('.').unwrap().1.to_lowercase()));
        }
        // 大写扩展名与小写同一类
        assert_eq!(classify_file("A.PDF", false), CATEGORY_DOC);
        assert_eq!(classify_file("B.PNG", false), CATEGORY_IMAGE);
    }

    #[test]
    fn unknown_ext_falls_to_other() {
        assert_class("a.xyz", CATEGORY_OTHER);
        assert_class("b.unknownext", CATEGORY_OTHER);
        assert_class("c.abcd", CATEGORY_OTHER);
    }

    #[test]
    fn no_ext_and_hidden_files_to_other() {
        // 无扩展名
        assert_class("README", CATEGORY_OTHER);
        assert_class("Makefile", CATEGORY_OTHER);
        // 隐藏文件:点开头但无扩展名(或扩展名不在规则表)
        assert_class(".gitignore", CATEGORY_OTHER);
        assert_class(".env", CATEGORY_OTHER);
        assert_class(".DS_Store", CATEGORY_OTHER);
    }

    #[test]
    fn dir_always_folder() {
        assert_eq!(classify_file("项目资料", true), CATEGORY_FOLDER);
        // 目录即使带扩展名也算文件夹(目录不看扩展名)
        assert_eq!(classify_file("photos.2024", true), CATEGORY_FOLDER);
    }

    #[test]
    fn empty_ext_is_other() {
        assert_eq!(classify_ext(""), CATEGORY_OTHER);
        assert_eq!(classify_ext("   "), CATEGORY_OTHER);
    }

    #[test]
    fn category_order_is_complete_and_unique() {
        assert_eq!(CATEGORY_ORDER.len(), 9);
        let mut seen = Vec::new();
        for c in CATEGORY_ORDER {
            assert!(!seen.contains(&c), "类别重复: {c}");
            seen.push(c);
        }
    }

    #[test]
    fn every_classify_result_is_in_order_table() {
        // 所有可能的返回值都必须是 CATEGORY_ORDER 中的合法类别
        for ext in ["pdf", "png", "mp4", "mp3", "zip", "exe", "rs", "zzz", ""] {
            let c = classify_ext(ext);
            assert!(CATEGORY_ORDER.contains(&c), "未知类别: {c}");
        }
        assert!(CATEGORY_ORDER.contains(&classify_file("某目录", true)));
    }
}
