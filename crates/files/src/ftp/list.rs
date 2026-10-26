//! O que o servidor conta de uma pasta: o MLSD (RFC 3659), o `LIST` no formato Unix e no do
//! DOS, e as datas (`MDTM`, `MFMT`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entry::{Entry, EntryKind};

/// Dias desde 1970-01-01 de uma data civil (algoritmo de Howard Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A data civil (ano, mês, dia) de um número de dias desde 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn from_unix(seconds: i64) -> SystemTime {
    if seconds >= 0 {
        UNIX_EPOCH + Duration::from_secs(seconds as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
    }
}

fn to_unix(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    }
}

/// `aaaammddhhmmss` (UTC), com fração opcional, de um `MDTM` ou do fato `modify` do MLSD.
pub(crate) fn parse_timestamp(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let digits = text.split('.').next()?;
    if digits.len() != 14 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| digits[range].parse::<u32>().ok();
    let year = i64::from(number(0..4)?);
    let (month, day) = (number(4..6)?, number(6..8)?);
    let (hour, minute, second) = (number(8..10)?, number(10..12)?, number(12..14)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(from_unix(
        days * 86_400 + i64::from(hour * 3600 + minute * 60 + second.min(60)),
    ))
}

/// `aaaammddhhmmss` (UTC) de um instante, para o `MFMT`.
pub(crate) fn format_timestamp(time: SystemTime) -> String {
    let seconds = to_unix(time);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Uma entrada do MLSD ou do MLST (`fato=valor;…; nome`). `None` nas linhas que não se
/// entendem e em `cdir`/`pdir`.
pub(crate) fn parse_mlsd_line(line: &str) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.trim_start_matches(' ').split_once(' ')?;
    let (mut kind, mut size, mut modified, mut mode) = (None, 0, None, None);
    let (mut owner, mut group, mut target) = (None, None, None);
    for fact in facts.split(';').filter(|fact| !fact.is_empty()) {
        let (key, value) = fact.split_once('=')?;
        match key.to_ascii_lowercase().as_str() {
            "type" => {
                let lower = value.to_ascii_lowercase();
                kind = Some(match lower.as_str() {
                    "file" => EntryKind::File,
                    "dir" => EntryKind::Dir,
                    "cdir" | "pdir" => return None,
                    other => match other.strip_prefix("os.unix=") {
                        Some(unix) if unix.starts_with("symlink") || unix.starts_with("slink") => {
                            // `OS.unix=slink:/alvo` (o proftpd).
                            target = value
                                .split_once(':')
                                .map(|(_, to)| to.to_owned())
                                .filter(|to| !to.is_empty());
                            EntryKind::Symlink
                        }
                        _ => EntryKind::Other,
                    },
                });
            }
            "size" | "sizd" => size = value.parse().unwrap_or(0),
            "modify" => modified = parse_timestamp(value),
            "unix.mode" => mode = u32::from_str_radix(value, 8).ok().map(|m| m & 0o7777),
            "unix.ownername" => owner = Some(value.to_owned()),
            "unix.owner" => {
                owner.get_or_insert_with(|| value.to_owned());
            }
            "unix.groupname" => group = Some(value.to_owned()),
            "unix.group" => {
                group.get_or_insert_with(|| value.to_owned());
            }
            "unix.target" => target = Some(value.to_owned()),
            _ => {}
        }
    }
    let mut entry = Entry::new(name, kind.unwrap_or(EntryKind::Other));
    entry.size = size;
    entry.modified = modified;
    entry.permissions = mode;
    entry.owner = owner;
    entry.group = group;
    entry.link_target = target;
    Some(entry)
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Separa os `count` primeiros campos (separados por espaços) e devolve o resto, que começa
/// logo depois do espaço que fechou o último campo (o nome pode começar com espaço).
fn split_fields(line: &str, count: usize) -> Option<(Vec<&str>, &str)> {
    let mut fields = Vec::with_capacity(count);
    let mut rest = line;
    for _ in 0..count {
        rest = rest.trim_start_matches(' ');
        let end = rest.find(' ')?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    Some((fields, rest.strip_prefix(' ').unwrap_or(rest)))
}

fn parse_mode(perms: &str) -> u32 {
    let bytes = perms.as_bytes();
    let mut mode = 0;
    for (index, &flag) in bytes.iter().enumerate().skip(1).take(9) {
        let bit = 8 - (index - 1);
        if matches!(flag, b'r' | b'w' | b'x' | b's' | b't') {
            mode |= 1 << bit;
        }
    }
    if matches!(bytes.get(3), Some(b's' | b'S')) {
        mode |= 0o4000;
    }
    if matches!(bytes.get(6), Some(b's' | b'S')) {
        mode |= 0o2000;
    }
    if matches!(bytes.get(9), Some(b't' | b'T')) {
        mode |= 0o1000;
    }
    mode
}

/// Uma linha do `LIST` no formato Unix (`drwxr-xr-x 2 dono grupo 4096 Oct 17 14:03 nome`).
/// `now` e `offset_min` servem às datas: sem ano, vale o atual (ou o anterior, se cair no
/// futuro); a hora vem no fuso do servidor, `offset_min` minutos a leste do UTC.
fn parse_unix_line(line: &str, now: SystemTime, offset_min: i32) -> Option<Entry> {
    let first = line.bytes().next()?;
    if !matches!(first, b'-' | b'd' | b'l' | b'c' | b'b' | b'p' | b's') {
        return None;
    }
    let (fields, name) = split_fields(line, 8)?;
    let perms = fields[0];
    if perms.len() < 10 || fields[1].parse::<u64>().is_err() {
        // Sem o número de links (alguns servidores): não é o formato Unix completo.
        return None;
    }
    let size: u64 = fields[4].parse().ok()?;
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(fields[5]))? as u32
        + 1;
    let day: u32 = fields[6].parse().ok()?;
    let modified = unix_date(month, day, fields[7], now, offset_min);
    let kind = match first {
        b'-' => EntryKind::File,
        b'd' => EntryKind::Dir,
        b'l' => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    let (name, link_target) = match (kind, name.split_once(" -> ")) {
        (EntryKind::Symlink, Some((name, target))) => (name, Some(target.to_owned())),
        _ => (name, None),
    };
    let mut entry = Entry::new(name, kind);
    entry.size = size;
    entry.modified = modified;
    entry.permissions = Some(parse_mode(perms));
    entry.owner = Some(fields[2].to_owned());
    entry.group = Some(fields[3].to_owned());
    entry.link_target = link_target;
    Some(entry)
}

fn unix_date(
    month: u32,
    day: u32,
    year_or_time: &str,
    now: SystemTime,
    offset_min: i32,
) -> Option<SystemTime> {
    let shift = i64::from(offset_min) * 60;
    if let Some((hour, minute)) = year_or_time.split_once(':') {
        let (hour, minute): (i64, i64) = (hour.parse().ok()?, minute.parse().ok()?);
        let (now_year, ..) = civil_from_days(to_unix(now).div_euclid(86_400));
        let at = |year| days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60;
        let mut seconds = at(now_year) - shift;
        // Mais de um dia no futuro: é do ano passado.
        if seconds > to_unix(now) + 86_400 {
            seconds = at(now_year - 1) - shift;
        }
        Some(from_unix(seconds))
    } else {
        let year: i64 = year_or_time.parse().ok()?;
        Some(from_unix(
            days_from_civil(year, month, day) * 86_400 - shift,
        ))
    }
}

/// Uma linha do `LIST` no formato do DOS/IIS (`10-17-23  02:03PM  <DIR>  nome`).
fn parse_dos_line(line: &str, offset_min: i32) -> Option<Entry> {
    let (fields, name) = split_fields(line, 3)?;
    let mut date = fields[0].split(['-', '/']);
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let year: i64 = date.next()?.parse().ok()?;
    let year = match year {
        0..=69 => 2000 + year,
        70..=99 => 1900 + year,
        _ => year,
    };
    let clock = fields[1].to_ascii_uppercase();
    let (time, pm) = match clock.strip_suffix("PM") {
        Some(time) => (time.to_owned(), true),
        None => (clock.trim_end_matches("AM").to_owned(), false),
    };
    let (hour, minute) = time.split_once(':')?;
    let (mut hour, minute): (i64, i64) = (hour.parse().ok()?, minute.parse().ok()?);
    if pm && hour < 12 {
        hour += 12;
    } else if !pm && clock.ends_with("AM") && hour == 12 {
        hour = 0;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60
        - i64::from(offset_min) * 60;
    let (kind, size) = if fields[2].eq_ignore_ascii_case("<DIR>") {
        (EntryKind::Dir, 0)
    } else {
        (EntryKind::File, fields[2].replace(',', "").parse().ok()?)
    };
    let mut entry = Entry::new(name.trim_start(), kind);
    entry.size = size;
    entry.modified = Some(from_unix(seconds));
    Some(entry)
}

/// Uma linha do `LIST` (Unix ou DOS). `None` no que não se entende (o `total 12`, as linhas em
/// branco).
pub(crate) fn parse_list_line(line: &str, now: SystemTime, offset_min: i32) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    parse_unix_line(line, now, offset_min).or_else(|| parse_dos_line(line, offset_min))
}

/// As entradas de um `LIST`, sem `.` e `..`.
pub(crate) fn parse_list(text: &str, now: SystemTime, offset_min: i32) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| parse_list_line(line, now, offset_min))
        .filter(|entry| entry.name != "." && entry.name != "..")
        .collect()
}

/// As entradas de um `MLSD`, sem `.` e `..`.
pub(crate) fn parse_mlsd(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(parse_mlsd_line)
        .filter(|entry| entry.name != "." && entry.name != "..")
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn agora() -> SystemTime {
        parse_timestamp("20261018120000").unwrap()
    }

    #[test]
    fn timestamp_vai_e_volta() {
        let tempo = parse_timestamp("20231017140305.123").unwrap();
        assert_eq!(format_timestamp(tempo), "20231017140305");
        assert_eq!(
            tempo.duration_since(UNIX_EPOCH).unwrap().as_secs(),
            1_697_551_385
        );
        assert!(parse_timestamp("2023").is_none());
    }

    #[test]
    fn mlsd_le_os_fatos() {
        let e = parse_mlsd_line(
            "type=file;size=1234;modify=20231017140305;UNIX.mode=0644;UNIX.owner=1000;UNIX.ownername=ana;UNIX.group=100; a b#.txt",
        )
        .unwrap();
        assert_eq!(e.name, "a b#.txt");
        assert_eq!((e.kind, e.size), (EntryKind::File, 1234));
        assert_eq!(e.permissions, Some(0o644));
        assert_eq!(e.owner.as_deref(), Some("ana"));
        assert_eq!(e.group.as_deref(), Some("100"));
        assert!(e.modified.is_some());
    }

    #[test]
    fn mlsd_pula_cdir_e_pdir_e_acha_links() {
        assert!(parse_mlsd_line("type=cdir;modify=20231017140305; .").is_none());
        assert!(parse_mlsd_line("Type=pdir; ..").is_none());
        let link = parse_mlsd_line("type=OS.unix=slink:/etc;size=4; atalho").unwrap();
        assert_eq!(link.kind, EntryKind::Symlink);
        assert_eq!(link.link_target.as_deref(), Some("/etc"));
    }

    #[test]
    fn list_unix_com_hora_e_com_ano() {
        let a = parse_list_line(
            "-rw-r--r--    1 ftp      ftp          4096 Oct 17 14:03 relatório final.txt",
            agora(),
            0,
        )
        .unwrap();
        assert_eq!(a.name, "relatório final.txt");
        assert_eq!(
            (a.kind, a.size, a.permissions),
            (EntryKind::File, 4096, Some(0o644))
        );
        assert_eq!(format_timestamp(a.modified.unwrap()), "20261017140300");
        let b = parse_list_line(
            "drwxr-xr-x 2 root root 4096 Dec  3  2019 antiga",
            agora(),
            0,
        )
        .unwrap();
        assert_eq!(b.kind, EntryKind::Dir);
        assert_eq!(format_timestamp(b.modified.unwrap()), "20191203000000");
    }

    #[test]
    fn list_sem_ano_no_futuro_vira_ano_passado() {
        let e = parse_list_line("-rw-r--r-- 1 a b 1 Dec 25 10:00 f", agora(), 0).unwrap();
        assert_eq!(format_timestamp(e.modified.unwrap()), "20251225100000");
    }

    #[test]
    fn list_aplica_o_fuso_do_servidor() {
        // O servidor está em UTC-3 (-180 min): 14:03 local = 17:03 UTC.
        let e = parse_list_line("-rw-r--r-- 1 a b 1 Oct 17 14:03 f", agora(), -180).unwrap();
        assert_eq!(format_timestamp(e.modified.unwrap()), "20261017170300");
    }

    #[test]
    fn list_link_modo_especial_e_total() {
        let l = parse_list_line(
            "lrwxrwxrwx 1 ftp ftp 11 Jan  1  2024 atalho -> /dados/real",
            agora(),
            0,
        )
        .unwrap();
        assert_eq!(l.name, "atalho");
        assert_eq!(l.link_target.as_deref(), Some("/dados/real"));
        let s = parse_list_line("-rwsr-xr-t 1 a b 1 Jan  1  2024 s", agora(), 0).unwrap();
        assert_eq!(s.permissions, Some(0o5755));
        assert!(parse_list_line("total 12", agora(), 0).is_none());
    }

    #[test]
    fn list_dos() {
        let d = parse_list_line(
            "10-17-23  02:03PM       <DIR>          Pasta nova",
            agora(),
            0,
        )
        .unwrap();
        assert_eq!((d.kind, d.name.as_str()), (EntryKind::Dir, "Pasta nova"));
        assert_eq!(format_timestamp(d.modified.unwrap()), "20231017140300");
        let f = parse_list_line("01-02-2024  12:30AM  1,234 a.txt", agora(), 0).unwrap();
        assert_eq!((f.kind, f.size), (EntryKind::File, 1234));
        assert_eq!(format_timestamp(f.modified.unwrap()), "20240102003000");
    }

    #[test]
    fn list_inteiro_descarta_pontos() {
        let texto = "total 8\r\ndrwxr-xr-x 2 a b 4096 Oct 17 14:03 .\r\ndrwxr-xr-x 2 a b 4096 Oct 17 14:03 ..\r\n-rw-r--r-- 1 a b 3 Oct 17 14:03 x\r\n";
        let lista = parse_list(texto, agora(), 0);
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].name, "x");
    }
}
