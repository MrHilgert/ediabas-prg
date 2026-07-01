//! Minimal example of the `ediabas` library API — the same flow a GUI would use.
//!
//!   cargo run --example read_measurements -- COM3 ecu/DDE40KW0.prg 0F300F65
//!
//! Reads two measurements (armM_List + anmUBT) from a BMW DDE4.0 in one batch.

use ediabas::Session;

fn main() -> Result<(), ediabas::Error> {
    let mut args = std::env::args().skip(1);
    let port = args.next().unwrap_or_else(|| "COM3".into());
    let prg = args.next().unwrap_or_else(|| "ecu/DDE40KW0.prg".into());
    let selectors = args.next().unwrap_or_else(|| "0F30".into());

    // 1) Open the adapter + load the ECU description.
    let mut session = Session::open(&port, 9600, &prg)?;

    // 2) Establish the protocol (present on nearly every BMW ECU).
    session.initialize()?;

    // 3) Run a job with structured results.
    let result = session.run_job("MW_SELECT_LESEN_NORM", &selectors)?;

    println!("JOB_STATUS = {:?}\n", result.job_status());
    for (i, set) in result.sets().iter().enumerate() {
        if set.is_empty() {
            continue;
        }
        println!("--- set {i} ---");
        for (name, value) in set.iter() {
            println!("  {name:<28} = {value}");
        }
    }

    // Or read one value directly by name:
    if let Some(v) = result.get_f64("STAT_armM_List_WERT") {
        println!("\narmM_List = {v} mg/Hub Luft");
    }

    Ok(())
}
