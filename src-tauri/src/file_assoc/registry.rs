//! 所有路径均相对注入的用户根；测试只传唯一的私有 HKCU 子键。
use super::*;
use std::{io, path::Path};
use winreg::{enums::*, RegKey};

const CLASSES: &str = "Software\\Classes";
const EXPLORER: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts";
const CAPS: &str = "Software\\MaobuFetch\\Capabilities";
const APPLICATION: &str = "Applications\\maobu-fetch.exe";

pub(super) fn ensure(root: &RegKey, exe: &Path) -> io::Result<()> {
    let classes = root.create_subkey(CLASSES)?.0;
    register_application(&classes, exe)?;
    let supported = classes
        .create_subkey(format!("{APPLICATION}\\SupportedTypes"))?
        .0;
    for extension in SUPPORTED_VIDEO_EXTS.iter().chain(SUPPORTED_IMAGE_EXTS) {
        supported.set_value(format!(".{extension}"), &"")?;
    }
    register_capabilities(root, exe)
}

pub(super) fn set(
    root: &RegKey,
    exe: &Path,
    extensions: &[String],
    enable: bool,
) -> io::Result<()> {
    let extensions = normalize_extensions(extensions)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    if extensions.is_empty() {
        return Ok(());
    }
    if enable {
        let classes = root.create_subkey(CLASSES)?.0;
        register_application(&classes, exe)?;
        register_capabilities(root, exe)?;
        for image in [false, true] {
            if extensions
                .iter()
                .any(|extension| is_image(extension) == image)
            {
                register_progid(&classes, exe, image)?;
            }
        }
        let capabilities = root.create_subkey(format!("{CAPS}\\FileAssociations"))?.0;
        let supported = classes
            .create_subkey(format!("{APPLICATION}\\SupportedTypes"))?
            .0;
        let explorer = root.create_subkey(EXPLORER)?.0;
        for extension in extensions {
            let dot = format!(".{extension}");
            let progid = progid(is_image(&extension));
            classes
                .create_subkey(format!("{dot}\\OpenWithProgids"))?
                .0
                .set_value(progid, &"")?;
            classes.create_subkey(format!("{dot}\\OpenWithList\\maobu-fetch.exe"))?;
            explorer
                .create_subkey(format!("{dot}\\OpenWithProgids"))?
                .0
                .set_value(progid, &"")?;
            capabilities.set_value(&dot, &progid)?;
            supported.set_value(&dot, &"")?;
        }
    } else {
        for extension in extensions {
            let dot = format!(".{extension}");
            let progid = progid(is_image(&extension));
            delete_value(root, &format!("{CLASSES}\\{dot}\\OpenWithProgids"), progid)?;
            delete_key(
                root,
                &format!("{CLASSES}\\{dot}\\OpenWithList\\maobu-fetch.exe"),
            )?;
            delete_value(root, &format!("{EXPLORER}\\{dot}\\OpenWithProgids"), progid)?;
            delete_value(root, &format!("{CAPS}\\FileAssociations"), &dot)?;
            delete_value(
                root,
                &format!("{CLASSES}\\{APPLICATION}\\SupportedTypes"),
                &dot,
            )?;
        }
    }
    Ok(())
}

pub(super) fn list(root: &RegKey) -> io::Result<Vec<FileAssocInfo>> {
    let Some(classes) = optional_key(root, CLASSES, KEY_READ)? else {
        return Ok(unassociated());
    };
    let mut result = Vec::new();
    for extension in SUPPORTED_VIDEO_EXTS.iter().chain(SUPPORTED_IMAGE_EXTS) {
        let image = is_image(extension);
        let associated = match optional_key(&classes, &format!(".{extension}"), KEY_READ)? {
            None => false,
            Some(key) => {
                let default = match key.get_value::<String, _>("") {
                    Ok(value) => value == progid(image),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                    Err(e) => return Err(e),
                };
                let candidate = match optional_key(&key, "OpenWithProgids", KEY_READ)? {
                    None => false,
                    Some(key) => match key.get_raw_value(progid(image)) {
                        Ok(_) => true,
                        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                        Err(e) => return Err(e),
                    },
                };
                default || candidate
            }
        };
        result.push(FileAssocInfo {
            extension: extension.to_string(),
            is_associated: associated,
            category: if image { "image" } else { "video" }.into(),
        });
    }
    Ok(result)
}

fn register_application(classes: &RegKey, exe: &Path) -> io::Result<()> {
    let exe = executable(exe)?;
    let application = classes.create_subkey(APPLICATION)?.0;
    application.set_value("FriendlyAppName", &"猫步下载器")?;
    application.set_value("ApplicationCompany", &"Maobu Fetch")?;
    application
        .create_subkey("DefaultIcon")?
        .0
        .set_value("", &format!("\"{exe}\",0"))?;
    application
        .create_subkey("shell\\open\\command")?
        .0
        .set_value("", &format!("\"{exe}\" \"%1\""))
}

fn register_capabilities(root: &RegKey, exe: &Path) -> io::Result<()> {
    let exe = executable(exe)?;
    let capabilities = root.create_subkey(CAPS)?.0;
    capabilities.set_value("ApplicationName", &"猫步下载器")?;
    capabilities.set_value("ApplicationDescription", &"猫步下载器与极速媒体看图器")?;
    capabilities.set_value("ApplicationIcon", &format!("\"{exe}\",0"))?;
    root.create_subkey("Software\\RegisteredApplications")?
        .0
        .set_value("MaobuFetch", &CAPS)
}

fn register_progid(classes: &RegKey, exe: &Path, image: bool) -> io::Result<()> {
    let exe = executable(exe)?;
    let key = classes.create_subkey(progid(image))?.0;
    key.set_value(
        "",
        &if image {
            IMAGE_PROG_DESCRIPTION
        } else {
            VIDEO_PROG_DESCRIPTION
        },
    )?;
    key.set_value(
        "FriendlyTypeName",
        &if image {
            "猫步看图器 图像文件"
        } else {
            "猫步播放器 媒体文件"
        },
    )?;
    key.set_value("AppUserModelId", &"app.lumaget.desktop")?;
    key.create_subkey("DefaultIcon")?
        .0
        .set_value("", &format!("\"{exe}\",0"))?;
    let argument = if image { "--view-image" } else { "--play" };
    key.create_subkey("shell\\open\\command")?
        .0
        .set_value("", &format!("\"{exe}\" {argument} \"%1\""))
}

fn executable(path: &Path) -> io::Result<&str> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "程序路径包含不受支持的字符"))?;
    if !path.is_absolute() || text.contains(['"', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "程序路径必须是明确的绝对路径",
        ));
    }
    Ok(text)
}

fn progid(image: bool) -> &'static str {
    if image {
        IMAGE_PROG_ID
    } else {
        VIDEO_PROG_ID
    }
}

fn optional_key(root: &RegKey, path: &str, flags: u32) -> io::Result<Option<RegKey>> {
    match root.open_subkey_with_flags(path, flags) {
        Ok(key) => Ok(Some(key)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn delete_value(root: &RegKey, path: &str, value: &str) -> io::Result<()> {
    let Some(key) = optional_key(root, path, KEY_SET_VALUE)? else {
        return Ok(());
    };
    missing_ok(key.delete_value(value))
}

fn delete_key(root: &RegKey, path: &str) -> io::Result<()> {
    missing_ok(root.delete_subkey(path))
}
fn missing_ok(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
