//! `cargo xtask data`: fetches the example data set and turns it into the three
//! CSV files that Chapter 25 loads.

use felite::read_record;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::process::Command;

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

/// Where the data comes from: US Bureau of Transportation Statistics, on-time
/// performance of reporting carriers. One archive for each month, with the
/// SHA-256 it had when the book was written.
const SOURCE: &str = "https://transtats.bts.gov/PREZIP/\
    On_Time_Reporting_Carrier_On_Time_Performance_1987_present_2024_";
const MONTHS: [(u32, &str); 2] = [
    (1, "fe089b45523f9d4ac0ccd0e176a543d274e914ee5ae5bd384dbaafc7dc06ebfd"),
    (2, "8463683d100504a97847497d2f2281008fd9add9639bf7436a9747032df15f9a"),
];
const CARRIER_NAMES: &str =
    "https://www.transtats.bts.gov/Download_Lookup.asp?Y11x72=Y_haVdhR_PNeeVRef";

/// How far apart the flights of the fixture are in January's data.
const FIXTURE_STRIDE: usize = 2736;

/// The SHA-256 digest of some bytes, as 64 hexadecimal digits.
pub fn sha256(input: &[u8]) -> String {
    // The constants are the first 32 bits of the fractional parts of the square
    // roots (for the starting state) and cube roots (for the rounds) of the
    // first primes.
    let primes: Vec<u32> =
        (2u32..).filter(|n| (2..*n).all(|d| n % d != 0)).take(64).collect();
    let fraction = |x: f64| ((x - x.floor()) * 4_294_967_296.0) as u32;
    let mut state = [0u32; 8];
    for (word, prime) in state.iter_mut().zip(&primes) {
        *word = fraction((*prime as f64).sqrt());
    }
    let rounds: Vec<u32> =
        primes.iter().map(|p| fraction((*p as f64).cbrt())).collect();

    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend((input.len() as u64 * 8).to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, bytes) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*bytes);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(rounds[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) =
                (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (total, part) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *total = total.wrapping_add(part);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

/// Runs a program and fails if it does.
fn run(program: &str, args: &[&str]) -> Outcome<()> {
    let status = Command::new(program).args(args).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(format!("{program} {} failed", args.join(" ")).into()),
        Err(error) => Err(format!("could not run {program}: {error}").into()),
    }
}

/// Downloads `url` to `path` with curl, unless the file is already there.
pub fn fetch(url: &str, path: &str) -> Outcome<()> {
    if std::fs::metadata(path).is_ok() {
        return Ok(());
    }
    println!("downloading {path}");
    run(
        "curl",
        &["--silent", "--show-error", "--location", "--fail", "--output", path, url],
    )
}

/// A whole number written the way the source writes one: `-5.00`, or nothing.
fn whole(field: &str) -> String {
    match field.parse::<f64>() {
        Ok(x) => format!("{}", x as i64),
        Err(_) => String::new(),
    }
}

/// A field for a CSV file, in quotes if it needs them.
fn quoted(field: &str) -> String {
    match field.contains([',', '"', '\n']) {
        true => format!("\"{}\"", field.replace('"', "\"\"")),
        false => field.to_string(),
    }
}

/// A value for the fixture's SQL: text in quotes, NULL for nothing.
fn sql(field: &str, text: bool) -> String {
    match (field.is_empty(), text) {
        (true, _) => "NULL".to_string(),
        (false, true) => format!("'{}'", field.replace('\'', "''")),
        (false, false) => field.to_string(),
    }
}

pub fn data() -> Outcome<()> {
    std::fs::create_dir_all("data")?;
    let mut flights = BufWriter::new(File::create("data/flights.csv")?);
    let mut airports: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut carriers: BTreeMap<String, usize> = BTreeMap::new();
    let mut fixture = Vec::new();
    let mut id = 0;
    for (month, expected) in MONTHS {
        let zip = format!("data/ontime_2024_{month}.zip");
        fetch(&format!("{SOURCE}{month}.zip"), &zip)?;
        let mut bytes = Vec::new();
        File::open(&zip)?.read_to_end(&mut bytes)?;
        if sha256(&bytes) != expected {
            return Err(format!(
                "{zip} is not the file this book was written with. The source \
                 revises its data, so the numbers you get may differ a little. \
                 Delete the check in xtask/src/data.rs to go on regardless."
            )
            .into());
        }
        let dir = format!("data/ontime_2024_{month}");
        std::fs::create_dir_all(&dir)?;
        // `unzip` where there is one, and `tar`, which reads zip files on macOS
        // and Windows, where there is not.
        if run("unzip", &["-o", "-q", &zip, "-d", &dir]).is_err() {
            run("tar", &["-xf", &zip, "-C", &dir])?;
        }
        let csv = std::fs::read_dir(&dir)?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|e| e == "csv"))
            .ok_or("the archive has no CSV file in it")?;
        println!("reading {}", csv.display());
        let mut input = BufReader::new(File::open(csv)?);
        let header = read_record(&mut input)?.ok_or("the CSV file is empty")?;
        let at = |name: &str| {
            header.iter().position(|h| h == name).ok_or(format!("no column {name}"))
        };
        let columns = [
            at("FlightDate")?,
            at("Reporting_Airline")?,
            at("Flight_Number_Reporting_Airline")?,
            at("Tail_Number")?,
            at("Origin")?,
            at("Dest")?,
            at("CRSDepTime")?,
            at("DepDelay")?,
            at("ArrDelay")?,
            at("Cancelled")?,
            at("AirTime")?,
            at("Distance")?,
        ];
        let places = [("Origin", "OriginCityName"), ("Dest", "DestCityName")];
        let places = [
            (at(places[0].0)?, at(places[0].1)?),
            (at(places[1].0)?, at(places[1].1)?),
        ];
        while let Some(row) = read_record(&mut input)? {
            id += 1;
            for (code, city) in places {
                let (city, state) =
                    row[city].rsplit_once(", ").unwrap_or((&row[city], ""));
                airports
                    .insert(row[code].clone(), (city.to_string(), state.to_string()));
            }
            *carriers.entry(row[columns[1]].clone()).or_default() += 1;
            let field = |i: usize| row[columns[i]].as_str();
            let miles =
                field(11).parse::<f64>().map_or(String::new(), |x| format!("{x:?}"));
            let values = [
                id.to_string(),
                field(0).to_string(),
                field(1).to_string(),
                whole(field(2)),
                field(3).to_string(),
                field(4).to_string(),
                field(5).to_string(),
                whole(field(6)),
                whole(field(7)),
                whole(field(8)),
                whole(field(9)),
                whole(field(10)),
                miles,
            ];
            let line: Vec<String> = values.iter().map(|v| quoted(v)).collect();
            writeln!(flights, "{}", line.join(","))?;
            if month == 1 && id % FIXTURE_STRIDE == 0 {
                let text = [
                    false, true, true, false, true, true, true, false, false, false,
                    false, false, false,
                ];
                let shown: Vec<String> =
                    values.iter().zip(text).map(|(v, t)| sql(v, t)).collect();
                fixture.push(format!(
                    "INSERT INTO flights VALUES ({});",
                    shown.join(", ")
                ));
            }
        }
    }
    flights.flush()?;

    fetch(CARRIER_NAMES, "data/carrier_names.csv")?;
    let mut names = BTreeMap::new();
    let mut input = BufReader::new(File::open("data/carrier_names.csv")?);
    while let Some(row) = read_record(&mut input)? {
        if let [code, name] = row.as_slice() {
            names.insert(code.clone(), name.clone());
        }
    }
    let mut out = BufWriter::new(File::create("data/carriers.csv")?);
    for code in carriers.keys() {
        let name =
            names.get(code).ok_or(format!("no name for the carrier {code}"))?;
        writeln!(out, "{},{}", quoted(code), quoted(name))?;
    }
    out.flush()?;
    let mut out = BufWriter::new(File::create("data/airports.csv")?);
    for (code, (city, state)) in &airports {
        writeln!(out, "{},{},{}", quoted(code), quoted(city), quoted(state))?;
    }
    out.flush()?;
    println!(
        "{id} flights, {} airports and {} carriers are in data/",
        airports.len(),
        carriers.len()
    );

    // The fixture that the tests use was cut from this data. Say whether it
    // still agrees.
    let checked_in = include_str!("../../felite/fixtures/flights.sql");
    let its_flights: Vec<&str> =
        checked_in.lines().filter(|l| l.contains("INTO flights")).collect();
    match its_flights == fixture {
        true => println!("the {} flights of the fixture match", fixture.len()),
        false => println!("the fixture's flights DO NOT match this data"),
    }
    Ok(())
}
