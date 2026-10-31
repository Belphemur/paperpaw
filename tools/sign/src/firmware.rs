//! The firmware's commands: signing keys, update files, whole flash
//! images signed apart, and the plugin crates an image has built in.

use std::io::Write;

use paperpaw_update::{AppInfo, Check, PLUGINS_AT, Plugins, TRAILER_LEN, Trailer};

use crate::elf::Elf;
use crate::secret_key;

/// The symbol under which a firmware's ELF file holds the fingerprint
/// native plugins must have (8 bytes, little-endian), as
/// `shells/xteink/src/native` exports it.
const FINGERPRINT_SYMBOL: &str = "PAPERPAW_NATIVE_FINGERPRINT";

pub fn key(file: &str) -> Result<(), String> {
    let mut out = new_key_file(file).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{file} exists: a key is never overwritten"),
        _ => format!("{file}: {e}"),
    })?;
    let pair = ed25519_compact::KeyPair::generate();
    let seed: [u8; 32] = *pair.sk.seed();
    writeln!(out, "{}", paperpaw_update::key_hex(&seed)).map_err(|e| format!("{file}: {e}"))?;
    let public = paperpaw_update::public_key(&seed);
    println!("public key:  {}", paperpaw_update::key_hex(&public));
    println!("fingerprint: {}", paperpaw_update::fingerprint(&public));
    Ok(())
}

/// A new file for a secret key: never one that is there (checked and made
/// at once), and readable by its owner alone (on Windows, as the folder
/// it is in lets).
fn new_key_file(file: &str) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)
}

pub fn sign_apart(image: &str, signature: &str, key_file: Option<&str>) -> Result<(), String> {
    let seed = secret_key(key_file)?;
    let data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
    let trailer = paperpaw_update::sign(&data, &seed).map_err(|e| format!("{image}: {e}"))?;
    std::fs::write(signature, trailer.to_bytes()).map_err(|e| format!("{signature}: {e}"))?;
    println!(
        "{signature}: {} bytes signed by {}",
        data.len(),
        paperpaw_update::fingerprint(&trailer.key)
    );
    Ok(())
}

pub fn check(image: &str, signature: &str) -> Result<(), String> {
    let data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
    let sig = std::fs::read(signature).map_err(|e| format!("{signature}: {e}"))?;
    let trailer = paperpaw_update::check_apart(&data, &sig).map_err(|e| format!("{image}: {e}"))?;
    println!("image:       {} bytes", data.len());
    println!(
        "signer:      {} ({})",
        paperpaw_update::fingerprint(&trailer.key),
        paperpaw_update::key_hex(&trailer.key)
    );
    println!("signature:   holds");
    Ok(())
}

pub fn sign(image: &str, update: &str, key_file: Option<&str>) -> Result<(), String> {
    let seed = secret_key(key_file)?;
    let mut data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
    let info = AppInfo::read(&data).map_err(|e| format!("{image}: {e}"))?;
    let trailer = paperpaw_update::sign(&data, &seed).map_err(|e| format!("{image}: {e}"))?;
    data.extend_from_slice(&trailer.to_bytes());
    std::fs::write(update, &data).map_err(|e| format!("{update}: {e}"))?;
    println!(
        "{update}: {} {}, signed by {}",
        info.project,
        info.version,
        paperpaw_update::fingerprint(&trailer.key)
    );
    Ok(())
}

pub fn show(update: &str) -> Result<(), String> {
    let data = std::fs::read(update).map_err(|e| format!("{update}: {e}"))?;
    let split = data
        .len()
        .checked_sub(TRAILER_LEN)
        .ok_or_else(|| format!("{update}: too short"))?;
    let trailer =
        Trailer::read(&data[split..], data.len() as u64).map_err(|e| format!("{update}: {e}"))?;
    let info = AppInfo::read(&data[..split]).map_err(|e| format!("{update}: {e}"))?;
    let mut check = Check::new(&trailer).map_err(|e| format!("{update}: {e}"))?;
    check.absorb(&data[..split]);
    let holds = check.finish();
    println!("firmware:    {} {}", info.project, info.version);
    println!("chip:        {}", info.chip);
    println!("image:       {} bytes", trailer.image_len);
    match plugins_of(&data[..split]) {
        Some(list) => println!("plugins:     {} built in", list.iter().count()),
        None => println!("plugins:     not listed"),
    }
    println!(
        "signer:      {} ({})",
        paperpaw_update::fingerprint(&trailer.key),
        paperpaw_update::key_hex(&trailer.key)
    );
    match holds {
        Ok(()) => println!("signature:   holds"),
        Err(e) => return Err(format!("{update}: {e}")),
    }
    Ok(())
}

/// The plugin list of an app image (or an update file, which begins with
/// one).
fn plugins_of(image: &[u8]) -> Option<Plugins> {
    Plugins::read(image.get(PLUGINS_AT..)?)
}

pub fn plugins(image: &str) -> Result<(), String> {
    let data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
    AppInfo::read(&data).map_err(|e| format!("{image}: {e}"))?;
    let list = plugins_of(&data).ok_or_else(|| format!("{image}: no list of plugins"))?;
    if list.is_empty() {
        return Err(format!("{image}: no plugins listed"));
    }
    print!("{}", list.text());
    Ok(())
}

pub fn fingerprint(elf: &str) -> Result<(), String> {
    let data = std::fs::read(elf).map_err(|e| format!("{elf}: {e}"))?;
    let fingerprint = fingerprint_of(&data).map_err(|e| format!("{elf}: {e}"))?;
    println!("{fingerprint:016x}");
    Ok(())
}

/// The fingerprint a firmware's ELF file names.
fn fingerprint_of(file: &[u8]) -> Result<u64, String> {
    let bytes = symbol_bytes(file, FINGERPRINT_SYMBOL, 8)
        .map_err(|e| format!("{e}: a firmware without code plugins"))?;
    let bytes: [u8; 8] = bytes.try_into().map_err(|_| "not 8 bytes")?;
    Ok(u64::from_le_bytes(bytes))
}

/// The `len` bytes of the object `name` in a firmware's ELF file.
fn symbol_bytes<'a>(file: &'a [u8], name: &str, len: usize) -> Result<&'a [u8], String> {
    let elf = Elf::parse(file)?;
    let (_, symbols) = elf.symbols()?.ok_or("no symbol table")?;
    let symbol = symbols
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("no {name}"))?;
    elf.sections
        .get(usize::from(symbol.shndx))
        .filter(|_| usize::try_from(symbol.size).is_ok_and(|size| size == len))
        .and_then(|section| {
            let at = usize::try_from(symbol.value.checked_sub(section.addr)?).ok()?;
            elf.data(section).ok()?.get(at..at.checked_add(len)?)
        })
        .ok_or_else(|| format!("{name} is not {len} bytes of the file"))
}

/// What a firmware's ELF file says of its table for plugins, as its
/// `PAPERPAW_NATIVE_ABI` keeps it (version 1): the plugin interface, the
/// size of its `Api`, each family's revision and the oldest it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Abi {
    interface: u16,
    api: u32,
    families: [u16; paperpaw_table::FAMILIES],
    min: [u16; paperpaw_table::FAMILIES],
}

const ABI_SYMBOL: &str = "PAPERPAW_NATIVE_ABI";
const ABI_WORDS: usize = 3 + 2 * paperpaw_table::FAMILIES;

fn abi_of(file: &[u8]) -> Result<Abi, String> {
    let bytes = symbol_bytes(file, ABI_SYMBOL, 4 * ABI_WORDS)?;
    let word = |i: usize| {
        let at = 4 * i;
        bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_le_bytes)
    };
    if word(0) != 1 {
        return Err(format!("{ABI_SYMBOL} of version {}, not 1", word(0)));
    }
    let small = |i: usize| {
        u16::try_from(word(i)).map_err(|_| format!("{ABI_SYMBOL}: word {i} is too large"))
    };
    let mut families = [0; paperpaw_table::FAMILIES];
    let mut min = [0; paperpaw_table::FAMILIES];
    for i in 0..paperpaw_table::FAMILIES {
        families[i] = small(3 + i)?;
        min[i] = small(3 + paperpaw_table::FAMILIES + i)?;
    }
    Ok(Abi {
        interface: small(1)?,
        api: word(2),
        families,
        min,
    })
}

/// A release's `-abi.txt`: the fingerprint (the first line, as before),
/// then the table's lines the store and the web flasher read.
fn abi_text(fingerprint: u64, abi: &Abi) -> String {
    let numbers = |list: &[u16]| {
        list.iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "{fingerprint:016x}\ninterface: {}\napi: {}\nfamilies: {}\nmin: {}\n",
        abi.interface,
        abi.api,
        numbers(&abi.families),
        numbers(&abi.min)
    )
}

pub fn abi(elf: &str) -> Result<(), String> {
    let data = std::fs::read(elf).map_err(|e| format!("{elf}: {e}"))?;
    let fingerprint = fingerprint_of(&data).map_err(|e| format!("{elf}: {e}"))?;
    let abi = abi_of(&data).map_err(|e| format!("{elf}: {e}"))?;
    print!("{}", abi_text(fingerprint, &abi));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elf::build::Builder;
    use crate::elf::{EM_XTENSA, ET_EXEC, SHF_ALLOC};

    #[test]
    fn the_firmware_names_its_fingerprint() {
        let fingerprint = 0x86fe_a46c_f2a7_188a_u64;
        let mut data = vec![0xAA; 4];
        data.extend_from_slice(&fingerprint.to_le_bytes());
        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0020, &data, 4);
        b.symbol(FINGERPRINT_SYMBOL, 0x3c00_0024, 8, rodata as u16, false);
        assert_eq!(fingerprint_of(&b.build()), Ok(fingerprint));

        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0020, &data, 4);
        b.symbol("main", 0x3c00_0020, 4, rodata as u16, false);
        assert!(fingerprint_of(&b.build()).unwrap_err().starts_with("no "));
        // Not in the file.
        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0020, &data, 4);
        b.symbol(FINGERPRINT_SYMBOL, 0x3c00_0030, 8, rodata as u16, false);
        assert!(fingerprint_of(&b.build()).is_err());
    }

    /// The table's lines, read as the site reads them
    /// (`paperpaw.org` `firmware.rs` `Table::read`): every key there,
    /// `interface` a u16, `api` a u32, eight u16 for each family list.
    fn site_reads(text: &str) -> Option<(u64, Abi)> {
        let mut lines = text.lines();
        let fingerprint = u64::from_str_radix(lines.next()?.split_whitespace().next()?, 16).ok()?;
        let mut keys = std::collections::BTreeMap::new();
        for line in lines {
            let (key, value) = line.split_once(':')?;
            keys.insert(key.trim(), value.trim());
        }
        let eight = |key: &str| -> Option<[u16; 8]> {
            let list: Vec<u16> = keys
                .get(key)?
                .split_whitespace()
                .map(|n| n.parse().ok())
                .collect::<Option<_>>()?;
            list.try_into().ok()
        };
        let abi = Abi {
            interface: keys.get("interface")?.parse().ok()?,
            api: keys.get("api")?.parse().ok()?,
            families: eight("families")?,
            min: eight("min")?,
        };
        Some((fingerprint, abi))
    }

    #[test]
    fn the_abi_file_says_the_firmwares_table_as_the_site_reads_it() {
        let fingerprint = 0x86fe_a46c_f2a7_188a_u64;
        let mut words = vec![1u32, 5, 72];
        words.extend([3, 0, 1, 0, 0, 0, 0, 0]);
        words.extend([1, 0, 1, 0, 0, 0, 0, 0]);
        let mut data = fingerprint.to_le_bytes().to_vec();
        data.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0000, &data, 4);
        b.symbol(FINGERPRINT_SYMBOL, 0x3c00_0000, 8, rodata as u16, false);
        b.symbol(
            ABI_SYMBOL,
            0x3c00_0008,
            4 * ABI_WORDS as u32,
            rodata as u16,
            false,
        );
        let file = b.build();
        let abi = abi_of(&file).unwrap();
        let text = abi_text(fingerprint_of(&file).unwrap(), &abi);
        assert_eq!(
            text,
            "86fea46cf2a7188a\ninterface: 5\napi: 72\nfamilies: 3 0 1 0 0 0 0 0\nmin: 1 0 1 0 0 0 0 0\n"
        );
        assert_eq!(site_reads(&text), Some((fingerprint, abi)));
        // The first line alone, as store-plugin.yml reads it.
        assert_eq!(text.lines().next(), Some("86fea46cf2a7188a"));
        // A record of another version, or of another length: not read.
        let mut later = data.clone();
        later[8] = 2;
        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0000, &later, 4);
        b.symbol(
            ABI_SYMBOL,
            0x3c00_0008,
            4 * ABI_WORDS as u32,
            rodata as u16,
            false,
        );
        assert!(abi_of(&b.build()).unwrap_err().contains("version 2"));
        let mut b = Builder::new(EM_XTENSA, ET_EXEC);
        let rodata = b.section(".rodata", SHF_ALLOC, 0x3c00_0000, &data, 4);
        b.symbol(ABI_SYMBOL, 0x3c00_0008, 8, rodata as u16, false);
        assert!(abi_of(&b.build()).is_err());
    }

    #[test]
    fn a_key_is_made_once_for_its_owner() {
        let dir = std::env::temp_dir().join(format!("paperpaw-sign-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret.key");
        let file = file.to_str().unwrap();
        let _ = std::fs::remove_file(file);
        key(file).unwrap();
        let first = std::fs::read_to_string(file).unwrap();
        assert!(secret_key(Some(file)).is_ok());
        // A second key never replaces the first.
        assert!(key(file).unwrap_err().contains("never overwritten"));
        assert_eq!(std::fs::read_to_string(file).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
