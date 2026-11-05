use iced::futures::channel::oneshot;
use repasta::{Clip, MAX_CLIP_BYTES};
use std::io::Read;
use std::process::{Command, Stdio};

/// Reads clipboard text. Images and formatted text are macOS-only for now.
pub async fn capture() -> Result<Clip, &'static str> {
    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .spawn(move || {
            let _ = sender.send(read().map(Clip::text));
        })
        .map_err(|_| "Could not start clipboard capture.")?;
    receiver.await.unwrap_or(Err("Clipboard capture stopped."))
}

fn read() -> Result<String, &'static str> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty())
        || std::env::var_os("WAYLAND_SOCKET").is_some();
    let args: &[&str] = if wayland {
        &["wl-paste", "--no-newline", "--type", "text"]
    } else {
        &["xclip", "-selection", "clipboard", "-out"]
    };
    let mut child = Command::new("timeout")
        .args(["--kill-after=1s", "2s"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start GNU timeout. Check the Linux setup instructions.")?;

    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("clipboard helper stdout is piped")
        .take((MAX_CLIP_BYTES + 1) as u64)
        .read_to_end(&mut bytes);

    let status = child.wait().map_err(|_| "Clipboard capture stopped.")?;
    if bytes.len() > MAX_CLIP_BYTES {
        return Err("Clipboard text exceeds 64 KiB.");
    }
    if read.is_err() || !status.success() {
        return Err(if wayland {
            "Could not read text. Check wl-paste and Wayland clipboard access."
        } else {
            "Could not read text. Check xclip and X11 clipboard access."
        });
    }
    String::from_utf8(bytes).map_err(|_| "Clipboard text is not UTF-8.")
}
