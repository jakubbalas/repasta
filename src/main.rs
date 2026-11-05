use repasta::{Clip, History, MAX_CLIP_BYTES};
use std::io::{self, BufRead, Read, Write};
use std::process::ExitCode;

const USAGE: &str = "Usage: repasta search [QUERY]\nRead one UTF-8 clip per line from stdin and print escaped matches, newest first.";

fn search(query: &str, mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut history = History::default();
    let mut record = Vec::new();

    loop {
        record.clear();
        // Allow a CRLF delimiter and one extra byte to detect an oversized line.
        if input
            .by_ref()
            .take((MAX_CLIP_BYTES + 3) as u64)
            .read_until(b'\n', &mut record)?
            == 0
        {
            break;
        }

        let clip = match record.strip_suffix(b"\n") {
            Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
            None => &record,
        };
        if clip.len() > MAX_CLIP_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "clip exceeds 64 KiB",
            ));
        }
        let clip = std::str::from_utf8(clip)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "clip is not UTF-8"))?;
        history.capture(clip);
    }

    for clip in history.search(query).filter_map(Clip::plain) {
        writeln!(output, "{clip:?}")?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let query = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [command] if command == "search" => "",
        [command, query] if command == "search" => match query.to_str() {
            Some(query) => query,
            None => {
                eprintln!("Query must be UTF-8.");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match search(query, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("repasta: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_escapes_terminal_controls_and_accepts_crlf() {
        let mut output = Vec::new();
        search("hit", &b"hit\x1b\r\nmiss\nhit"[..], &mut output).unwrap();
        assert_eq!(output, b"\"hit\"\n\"hit\\u{1b}\"\n");
    }

    #[test]
    fn oversized_input_fails_without_printing_partial_history() {
        let mut input = b"first\n".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_CLIP_BYTES + 3));
        let mut output = Vec::new();
        assert_eq!(
            search("", &input[..], &mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.is_empty());
    }
}
