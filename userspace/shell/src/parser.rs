//! Plain ASCII shell command classification, separate from editing and authority.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command<'a> {
    Empty,
    Help,
    Uname,
    Net,
    Mem,
    Clear,
    Echo(&'a [u8]),
    Ls(&'a [u8]),
    Cat(&'a [u8]),
    Stat(&'a [u8]),
    Touch(&'a [u8]),
    Write {
        path: &'a [u8],
        text: &'a [u8],
        append: bool,
    },
    Mkdir(&'a [u8]),
    Remove(&'a [u8]),
    RunInit {
        hold: bool,
    },
    Ps,
    Kill(&'a [u8]),
    Wait(&'a [u8]),
    Unknown,
}

pub fn parse(line: &[u8]) -> Command<'_> {
    let (verb, argument) = if let Some(space) = line.iter().position(|byte| *byte == b' ') {
        (&line[..space], Some(&line[space + 1..]))
    } else {
        (line, None)
    };
    match (verb, argument) {
        (b"", None) => Command::Empty,
        (b"help", None) => Command::Help,
        (b"uname", None) => Command::Uname,
        (b"net", None) => Command::Net,
        (b"mem", None) => Command::Mem,
        (b"clear", None) => Command::Clear,
        (b"ls", None) => Command::Ls(b"/"),
        (b"ps", None) => Command::Ps,
        (b"echo", Some(text)) => Command::Echo(text),
        (b"ls", Some(path)) => Command::Ls(path),
        (b"cat", Some(path)) => Command::Cat(path),
        (b"stat", Some(path)) => Command::Stat(path),
        (b"touch", Some(path)) => Command::Touch(path),
        (b"write", Some(args)) => {
            let (path, text) = split_argument(args);
            Command::Write {
                path,
                text,
                append: false,
            }
        }
        (b"append", Some(args)) => {
            let (path, text) = split_argument(args);
            Command::Write {
                path,
                text,
                append: true,
            }
        }
        (b"mkdir", Some(path)) => Command::Mkdir(path),
        (b"rm", Some(path)) => Command::Remove(path),
        (b"run", Some(b"init")) => Command::RunInit { hold: false },
        (b"run", Some(b"init hold")) => Command::RunInit { hold: true },
        (b"kill", Some(id)) => Command::Kill(id),
        (b"wait", Some(id)) => Command::Wait(id),
        _ => Command::Unknown,
    }
}

fn split_argument(input: &[u8]) -> (&[u8], &[u8]) {
    let Some(space) = input.iter().position(|byte| *byte == b' ') else {
        return (input, &[]);
    };
    let mut text_start = space;
    while input.get(text_start) == Some(&b' ') {
        text_start += 1;
    }
    (&input[..space], &input[text_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_preserves_plain_argument_bytes_and_rejects_unadvertised_syntax() {
        assert_eq!(parse(b"ls"), Command::Ls(b"/"));
        assert_eq!(
            parse(b"write /USER/A.TXT  hello  world"),
            Command::Write {
                path: b"/USER/A.TXT",
                text: b"hello  world",
                append: false
            }
        );
        assert_eq!(
            parse(b"append A x"),
            Command::Write {
                path: b"A",
                text: b"x",
                append: true
            }
        );
        assert_eq!(parse(b"run init hold"), Command::RunInit { hold: true });
        assert_eq!(parse(b"run init other"), Command::Unknown);
        assert_eq!(parse(b"echo\tunsafe"), Command::Unknown);
        assert_eq!(parse(b""), Command::Empty);
    }
}
