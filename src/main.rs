use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

use ediabas::{driver, prg, transport, vm};
use driver::Driver as _;
use transport::Transport as _;

#[derive(Parser)]
#[command(name = "ediabas-prg", about = "EDIABAS .prg file parser and BEST/2 VM")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show file header info and ECU metadata
    Info { file: PathBuf },

    /// List all jobs defined in the file
    Jobs { file: PathBuf },

    /// Run a job against a simulated ECU using an EDIABAS .SIM file
    Sim {
        #[arg(short, long)]
        sim: PathBuf,
        #[arg(short, long)]
        request: String,
    },

    /// Execute a job from a .prg file using the BEST/2 VM
    Run {
        #[arg(long)]
        prg: PathBuf,
        job: String,
        #[arg(short, long)]
        port: Option<String>,
        #[arg(short, long, default_value = "9600")]
        baud: u32,
        /// Perform ISO 9141-2 K-line 5-baud init before running the job
        #[arg(long, value_name = "HEX_ADDR")]
        kline_init: Option<String>,
        /// Run this init job first (e.g. INITIALISIERUNG)
        #[arg(long, value_name = "JOB_NAME")]
        init_job: Option<String>,
        /// Enable TX echo reading (only if adapter mirrors TX back on RX)
        #[arg(long)]
        echo: bool,
        /// LEN byte position in ECU response: 1=concept-0x0006, 2=concept-0x0001
        #[arg(long, default_value = "1")]
        len_offset: usize,
        /// Inter-byte TX delay (ms). DDE4.0/classic DS2 need ~5ms (ParInterbyteTime).
        #[arg(long, default_value = "5")]
        interbyte: u64,
        /// Optional job arguments (EDIABAS ArgString), ';'-separated, e.g. --args "1B;2A;3C".
        /// Read by the job via par*/pary. Optional even if the job expects args.
        #[arg(long, value_name = "A;B;C")]
        args: Option<String>,
    },

    /// Send a K-line telegram via KDCAN STD:OBD adapter protocol
    ///
    /// The KDCAN adapter speaks STD:OBD (not raw K-line) over the serial port.
    /// This command sends SETECUCOMM + SETTELPARAMETER + SENDTELGRAM, then reads
    /// the adapter response which contains the full ECU K-line reply.
    Obd {
        #[arg(short, long)]
        port: String,
        #[arg(short, long, default_value = "9600")]
        baud: u32,
        /// ECU address byte (hex), e.g. B8 for DDE4.0
        #[arg(long, default_value = "B8")]
        ecu: String,
        /// Frame payload WITHOUT checksum, e.g. "B8 12 F1 01 A2"
        frame: String,
    },

    /// Passively listen on K-line and dump all received bytes (no TX)
    Listen {
        /// Serial port, e.g. /dev/ttyUSB0
        #[arg(short, long)]
        port: String,
        #[arg(short, long, default_value = "9600")]
        baud: u32,
        #[arg(long, default_value = "even", value_parser = ["even", "none"])]
        parity: String,
        /// Max silence between frames (ms) before printing separator
        #[arg(long, default_value = "50")]
        gap: u64,
    },

    /// Send raw DS2 bytes directly to ECU — bypasses the VM for transport debugging
    Raw {
        /// Serial port, e.g. /dev/ttyUSB0
        #[arg(short, long)]
        port: String,
        /// Baud rate
        #[arg(short, long, default_value = "9600")]
        baud: u32,
        /// Serial parity: even (BMW DS2 default) or none
        #[arg(long, default_value = "even", value_parser = ["even", "none"])]
        parity: String,
        /// Enable TX echo reading (only if adapter mirrors TX back on RX)
        #[arg(long)]
        echo: bool,
        /// Timeout waiting for first response byte (ms)
        #[arg(long, default_value = "2000")]
        timeout: u64,
        /// Inter-byte silence that signals end of frame (ms)
        #[arg(long, default_value = "20")]
        gap: u64,
        /// Init frame to send first (hex), e.g. "B8 06 F1 01 A2 00"
        #[arg(long)]
        init: Option<String>,
        /// Send K-line wakeup break pulse before the frame (ms LOW + ms idle)
        #[arg(long, default_value = "0")]
        wakeup_ms: u64,
        /// Half-duplex RTS direction control: set RTS=LOW after TX so ECU can drive K-line.
        /// Use when cable uses RTS as TX-enable (RTS=HIGH→TX driver active, RTS=LOW→bus released).
        #[arg(long)]
        rts_rx_low: bool,
        /// Force RTS to this level at open and keep it there (high/low). Overrides rts_rx_low initial state.
        #[arg(long, value_parser = ["high", "low"])]
        rts: Option<String>,
        /// Force DTR to this level at open (high/low). Default: high.
        #[arg(long, value_parser = ["high", "low"])]
        dtr: Option<String>,
        /// Extra wait (ms) between end of TX/echo and start of reading ECU response
        #[arg(long, default_value = "0")]
        regen_ms: u64,
        /// Inter-byte delay (ms) between each TX byte. DDE4.0 needs ~4ms (ParInterbyteTime).
        #[arg(long, default_value = "0")]
        interbyte: u64,
        /// Frame bytes to send (hex), e.g. "B8 12 F1 04 2C 10 0F 10"
        frame: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let result: Result<(), Box<dyn std::error::Error>> = match cli.command {
        Command::Info { file } => {
            prg::PrgFile::open(&file)
                .unwrap_or_else(|e| { eprintln!("Error: {e}"); std::process::exit(1); })
                .print_info()
        }
        Command::Jobs { file } => {
            prg::PrgFile::open(&file)
                .unwrap_or_else(|e| { eprintln!("Error: {e}"); std::process::exit(1); })
                .print_jobs()
        }
        Command::Sim { sim: sim_path, request } => {
            let mut ecu = transport::sim::SimTransport::load(&sim_path)?;
            let frame = parse_hex(&request).ok_or("invalid request hex")?;
            println!("Request : {}", fmt_hex(&frame));
            match ecu.exchange(&frame) {
                Ok(resp) => println!("Response: {}", fmt_hex(&resp)),
                Err(e)   => { eprintln!("No match: {e}"); std::process::exit(1); }
            }
            Ok(())
        }
        Command::Run { prg: prg_path, job, port, baud, kline_init, init_job, echo, len_offset, interbyte, args } => {
            let arg_buf: Vec<u8> = args.unwrap_or_default().into_bytes();
            let prg_file = prg::PrgFile::open(&prg_path).unwrap_or_else(|e| {
                eprintln!("Error opening .prg: {e}"); std::process::exit(1);
            });
            let code = prg_file.job_code(&job).unwrap_or_else(|| {
                eprintln!("Job '{}' not found in {}", job, prg_path.display());
                std::process::exit(1);
            });
            let tables = prg_file.parse_tables();

            let sets = match port {
                Some(dev) => {
                    let parity = serialport::Parity::Even;
                    let serial = driver::serial::SerialDriver::open_parity(&dev, baud, parity)
                        .unwrap_or_else(|e| {
                            eprintln!("Cannot open {dev}: {e}"); std::process::exit(1);
                        });
                    let mut ds2 = transport::ds2::Ds2Transport::new(Box::new(serial));
                    // KDCAN/FTDI K-line adapters mirror TX back on RX — always drain the
                    // echo so it can't be mistaken for (or shift) the ECU response.
                    // (`--echo` kept for compatibility; draining is now unconditional.)
                    let _ = echo;
                    ds2.echo = true;
                    ds2.len_offset = len_offset;
                    ds2.interbyte_ms = interbyte;

                    if let Some(hex_addr) = kline_init {
                        let addr = u8::from_str_radix(hex_addr.trim_start_matches("0x"), 16)
                            .unwrap_or_else(|_| {
                                eprintln!("Invalid --kline-init: {hex_addr}"); std::process::exit(1);
                            });
                        eprintln!("K-line 5-baud init addr={addr:#04x}...");
                        ds2.kline_5baud_init(addr).unwrap_or_else(|e| {
                            eprintln!("K-line init failed: {e}"); std::process::exit(1);
                        });
                    }

                    let mut vm = vm::Vm::new(Box::new(ds2), tables);
                    vm.set_args(arg_buf.clone());

                    // Run the init job to configure the protocol (len_offset/len_add/
                    // interbyte from CommParameter). Default to INITIALISIERUNG when the
                    // .prg has it and none was given — most DS2 ECUs need it for correct
                    // frame-length parsing (concept-6: len_offset=3, len_add=5).
                    let init_name = init_job.or_else(||
                        prg_file.job_code("INITIALISIERUNG").map(|_| "INITIALISIERUNG".to_string())
                    );
                    if let Some(init_name) = init_name {
                        let init_code = prg_file.job_code(&init_name).unwrap_or_else(|| {
                            eprintln!("Init job '{}' not found", init_name); std::process::exit(1);
                        });
                        vm.run_job(&init_code).unwrap_or_else(|e| {
                            eprintln!("Init job failed: {e}"); std::process::exit(1);
                        });
                    }

                    vm.run_job(&code).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                }
                None => {
                    let mut vm = vm::Vm::new(Box::new(transport::sim::NullTransport), tables);
                    vm.set_args(arg_buf.clone());
                    vm.run_job(&code).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                }
            };

            print_result_sets(&sets);
            Ok(())
        }
        Command::Obd { port, baud, ecu, frame } => {
            use std::io::{Read as _, Write as _};
            let ecu_byte = u8::from_str_radix(ecu.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| { eprintln!("bad --ecu hex"); std::process::exit(1); });
            let payload = parse_hex(&frame).ok_or("invalid frame hex")?;

            let parity = serialport::Parity::Even;
            let mut port_dev = serialport::new(&port, baud)
                .parity(parity)
                .stop_bits(serialport::StopBits::One)
                .data_bits(serialport::DataBits::Eight)
                .flow_control(serialport::FlowControl::None)
                .timeout(std::time::Duration::from_millis(500))
                .open()
                .map_err(|e| format!("Cannot open {port}: {e}"))?;
            eprintln!("Port {} @ {} baud even parity", port, baud);

            // --- SETECUCOMM (CMD=05): concept=0x0006, baud=9600=0x2580, ECU_addr, timeout=1000=0x03E8 ---
            let mut setecucomm = vec![
                0x05u8, 0x15, 0x00,          // CMD, LEN=21
                0x06, 0x00,                   // concept 0x0006
                0x80, 0x25,                   // baud 9600 little-endian
                ecu_byte,                     // ECU addr (B8)
                0x00, 0x00, 0x00, 0x00, 0x00, // padding
                0xE8, 0x03,                   // timeout 1000ms
                0x64, 0x00,                   // T_ECU_REGENERATION 100ms
                0x32, 0x00,                   // T_WUP_TELEGRAM 50ms
                0x04, 0x00,                   // WAKEUP_TYPE 4
            ];
            eprintln!("SETECUCOMM TX: {}", fmt_hex(&setecucomm));
            port_dev.write_all(&setecucomm)?;
            port_dev.set_timeout(std::time::Duration::from_millis(500))?;
            let mut ack = [0u8; 3];
            port_dev.read_exact(&mut ack).ok();
            eprintln!("SETECUCOMM ACK: {}", fmt_hex(&ack));

            // --- SETTELPARAMETER (CMD=14): CommAnswerLen FD FF 05 ---
            let settel = [0x14u8, 0x06, 0x00, 0xFD, 0xFF, 0x05];
            eprintln!("SETTEL TX: {}", fmt_hex(&settel));
            port_dev.write_all(&settel)?;
            port_dev.read_exact(&mut ack).ok();
            eprintln!("SETTEL ACK: {}", fmt_hex(&ack));

            // --- SENDTELGRAM (CMD=06): payload WITHOUT checksum ---
            let total = 3u8 + payload.len() as u8;
            let mut send_msg = vec![0x06u8, total, 0x00];
            send_msg.extend_from_slice(&payload);
            eprintln!("SENDTEL TX: {}", fmt_hex(&send_msg));
            port_dev.write_all(&send_msg)?;

            // Adapter responds: [01][LEN_L][LEN_H][K-line response including CHK]
            port_dev.set_timeout(std::time::Duration::from_millis(3000))?;
            let mut hdr = [0u8; 3];
            port_dev.read_exact(&mut hdr)?;
            eprintln!("RESP HDR: {}", fmt_hex(&hdr));
            if hdr[0] == 0x01 {
                let resp_len = u16::from_le_bytes([hdr[1], hdr[2]]) as usize - 3;
                let mut resp = vec![0u8; resp_len];
                port_dev.read_exact(&mut resp)?;
                eprintln!("RESP ({} bytes): {}", resp.len(), fmt_hex(&resp));
                analyze_ds2_frame(&resp);
            } else {
                eprintln!("Unexpected response: {}", fmt_hex(&hdr));
            }
            Ok(())
        }

        Command::Listen { port, baud, parity, gap } => {
            use std::time::Instant;
            let parity_val = match parity.as_str() {
                "none" => serialport::Parity::None,
                _      => serialport::Parity::Even,
            };
            let serial = driver::serial::SerialDriver::open_parity(&port, baud, parity_val)
                .unwrap_or_else(|e| {
                    eprintln!("Cannot open {port}: {e}"); std::process::exit(1);
                });
            let mut ds2 = transport::ds2::Ds2Transport::new(Box::new(serial));
            eprintln!("Listening on {} @ {} baud, parity={} (gap={}ms). Ctrl-C to stop.", port, baud, parity, gap);
            let mut frame_num = 0u32;
            loop {
                // Wait up to 30s for first byte of next frame
                let bytes = ds2.receive_raw(30_000, gap);
                if bytes.is_empty() {
                    eprint!(".");
                    continue;
                }
                frame_num += 1;
                let now = Instant::now();
                eprintln!("\n[frame {}] {} bytes: {}", frame_num, bytes.len(), fmt_hex(&bytes));
                analyze_ds2_frame(&bytes);
                let _ = now;
            }
        }

        Command::Raw { port, baud, parity, echo, timeout, gap, init, wakeup_ms,
                       rts_rx_low, rts, dtr, regen_ms, interbyte, frame } => {
            let parity_val = match parity.as_str() {
                "none" => serialport::Parity::None,
                _      => serialport::Parity::Even,
            };

            let mut serial = driver::serial::SerialDriver::open_parity(&port, baud, parity_val)
                .unwrap_or_else(|e| {
                    eprintln!("Cannot open {port}: {e}"); std::process::exit(1);
                });

            // Apply explicit RTS/DTR overrides before anything else
            if let Some(rts_str) = &rts {
                let val = rts_str == "high";
                serial.set_rts(val).unwrap_or_else(|e| eprintln!("[warn] set_rts: {e}"));
                eprintln!("RTS forced {}", rts_str);
            }
            if let Some(dtr_str) = &dtr {
                let val = dtr_str == "high";
                serial.set_dtr(val).unwrap_or_else(|e| eprintln!("[warn] set_dtr: {e}"));
                eprintln!("DTR forced {}", dtr_str);
            }
            // Drain any stale bytes in the FT232R FIFO
            serial.flush_rx();

            let mut ds2 = transport::ds2::Ds2Transport::new(Box::new(serial));
            ds2.echo = echo;
            ds2.regen_delay_ms = regen_ms;
            ds2.interbyte_ms = interbyte;
            if rts_rx_low {
                ds2.rts_rx_low = Some(true);
                eprintln!("RTS half-duplex: HIGH during TX, LOW during RX");
            }

            eprintln!("Port {} @ {} baud, parity={}, echo={}, regen_ms={}, interbyte_ms={}", port, baud, parity, echo, regen_ms, interbyte);

            if wakeup_ms > 0 {
                eprintln!("Wakeup: BREAK {}ms + idle {}ms", wakeup_ms, wakeup_ms);
                ds2.fast_wakeup(wakeup_ms)?;
                eprintln!("Wakeup done, sending frame...");
            }

            if let Some(init_hex) = init {
                let bytes = parse_hex(&init_hex).ok_or("invalid --init hex")?;
                let with_chk = transport::ds2::Ds2Transport::append_checksum(&bytes);
                eprintln!("INIT TX: {}", fmt_hex(&with_chk));
                ds2.send_raw(&with_chk)?;
                let resp = ds2.receive_raw(timeout, gap);
                if resp.is_empty() {
                    eprintln!("INIT RX: (no response)");
                } else {
                    eprintln!("INIT RX: {}", fmt_hex(&resp));
                    analyze_ds2_frame(&resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            let bytes = parse_hex(&frame).ok_or("invalid frame hex")?;
            let with_chk = transport::ds2::Ds2Transport::append_checksum(&bytes);
            eprintln!("TX: {}", fmt_hex(&with_chk));
            ds2.send_raw(&with_chk)?;

            let raw = ds2.receive_raw(timeout, gap);
            if raw.is_empty() {
                eprintln!("RX: (timeout — no response)");
                std::process::exit(1);
            }

            eprintln!("RX ({} bytes): {}", raw.len(), fmt_hex(&raw));
            analyze_ds2_frame(&raw);
            Ok(())
        }
    };
    std::io::stdout().flush().ok();
    result
}

/// Print byte-annotated DS2 frame analysis.
/// Tries all known header variants:
///   offset=1, len_add=0 → [ADDR][LEN=total][SVC][DATA...][CHK]
///   offset=2, len_add=0 → [ADDR][SRC][LEN=total][SVC][DATA...][CHK]
///   offset=3, len_add=5 → [FMT=B8][TGT][SRC][LEN=payload_count][SVC][DATA...][CHK]
fn analyze_ds2_frame(raw: &[u8]) {
    eprintln!("--- frame analysis ---");
    for (offset, len_add) in [(1usize, 0usize), (2, 0), (3, 5)] {
        if raw.len() <= offset { continue; }
        let len_byte = raw[offset] as usize;
        let total = len_byte + len_add;
        let hdr_len = offset + 1;
        if total < hdr_len + 1 || total == 0 { continue; }
        eprint!("  [LEN@[{offset}]+{len_add}] LEN={len_byte:#04x}({len_byte}d) total={total}  ");
        if total > raw.len() {
            eprintln!("→ incomplete ({} of {total} bytes)", raw.len());
        } else if total < raw.len() {
            eprintln!("→ shorter than received ({total} < {})", raw.len());
        } else {
            let expected = raw[..total - 1].iter().fold(0u8, |a, &b| a ^ b);
            let actual   = raw[total - 1];
            if expected == actual {
                eprintln!("→ CHK OK ✓");
                let payload = &raw[hdr_len..total - 1];
                eprintln!("  payload: {}", fmt_hex(payload));
                if !payload.is_empty() {
                    eprintln!("  SVC={:#04x} data: {}", payload[0], fmt_hex(&payload[1..]));
                }
            } else {
                eprintln!("→ CHK FAIL (exp {expected:#04x} got {actual:#04x})");
            }
        }
    }
    eprintln!("  raw bytes:");
    for (i, b) in raw.iter().enumerate() {
        eprintln!("    [{i:2}] {b:#04x} ({b:3})  '{}'",
            if b.is_ascii_graphic() { *b as char } else { '.' });
    }
}

fn print_result_sets(sets: &[vm::ResultSet]) {
    for (i, set) in sets.iter().enumerate() {
        if sets.len() > 1 { println!("--- Result set {} ---", i + 1); }
        let mut keys: Vec<&String> = set.keys().collect();
        keys.sort();
        for k in keys { println!("{:<30} = {}", k, set[k]); }
    }
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    transport::ds2::parse_hex(s)
        .or_else(|| s.split(',').map(|t| u8::from_str_radix(t.trim(), 16).ok()).collect())
}

fn fmt_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
