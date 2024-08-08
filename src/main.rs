use core::time;
use std::path::PathBuf;

use clap::Parser;

const NUM_ITERATIONS: u32 = 10_000;

#[derive(Debug, Parser)]
#[command(
    author = "RunEasy",
    about = "A micro-benchmark program to measure the performance of hr_sleep and nanosleep"
)]
struct Args {
    #[arg(
        short,
        long,
        default_value_t = 0,
        help = "The core of the benchmark thread binding"
    )]
    core: u32,
    #[arg(
        short,
        long,
        default_value_t = 800,
        help = "The frequency of the core in MHz"
    )]
    freq: u32,
}

fn main() {
    let arg = Args::parse();

    let mut setup_succ = true;
    let mut old_governor = None;
    let mut old_cpufreq = None;

    if !core_available(arg.core) && setup_succ {
        eprintln!("The core {} is not available", arg.core);
        setup_succ = false;
    }

    if !acpi_avaiable(arg.core) && setup_succ {
        eprintln!(
            "The scaling driver of core {} is not acpi-cpufreq",
            arg.core
        );
        setup_succ = false;
    }

    if !cpufreq_available(arg.core, arg.freq * 1000) && setup_succ {
        setup_succ = false;
    }

    if setup_succ {
        old_governor = acpi_set_governor(arg.core, "userspace");
        if old_governor.is_none() {
            setup_succ = false;
        }
    }

    if setup_succ {
        old_cpufreq = acpi_set_freq(arg.core, arg.freq * 1000);
        if old_cpufreq.is_none() {
            setup_succ = false;
        }
    }

    if setup_succ {
        // start the benchmark.
        loop {
            if !bind_core(arg.core) {
                break;
            }

            const TEST_CASES: [std::time::Duration; 6] = [
                std::time::Duration::from_micros(1),
                std::time::Duration::from_micros(5),
                std::time::Duration::from_micros(10),
                std::time::Duration::from_micros(50),
                std::time::Duration::from_micros(100),
                std::time::Duration::from_micros(200),
            ];

            println!("Benchmark Options: ");
            println!("  Core: {}", arg.core);
            println!("  Frequency: {} MHz", arg.freq);
            println!("  Turbo boost: off");
            println!("");

            let mut case_results: Vec<(f64, f64, f64, f64)> = Vec::new();
            let tsc_per_micro = rtsc_time::cycles_per_sec() as f64 / 1_000_000.0;
            for case in TEST_CASES {
                let hr_sleep_results = hr_sleep_bench(case);
                let nano_sleep_results = nano_sleep_bench(case);
                let hr_sleep_results_mean = hr_sleep_results.iter().sum::<u64>() as f64
                    / (hr_sleep_results.len() as f64 * tsc_per_micro);
                let hr_sleep_results_99p =
                    hr_sleep_results[hr_sleep_results.len() * 99 / 100] as f64 / tsc_per_micro;
                let nano_sleep_results_mean = nano_sleep_results.iter().sum::<u64>() as f64
                    / (nano_sleep_results.len() as f64 * tsc_per_micro);
                let nano_sleep_results_99p =
                    nano_sleep_results[nano_sleep_results.len() * 99 / 100] as f64 / tsc_per_micro;
                case_results.push((
                    hr_sleep_results_mean,
                    hr_sleep_results_99p,
                    nano_sleep_results_mean,
                    nano_sleep_results_99p,
                ));
            }

            println!("Benchmark result: ");
            println!("                hr_sleep              nanosleep");
            for (i, case) in TEST_CASES.iter().enumerate() {
                println!(
                    "{}ns        {:.2}ns/{:.2}ns     {:.2}ns/{:.2}ns",
                    case.as_micros(),
                    case_results[i].0,
                    case_results[i].1,
                    case_results[i].2,
                    case_results[i].3
                );
            }

            break;
        }
    }

    if let Some(old_freq) = old_cpufreq {
        acpi_set_freq(arg.core, old_freq);
    }

    if let Some(old_governor) = old_governor {
        acpi_set_governor(arg.core, old_governor);
    }

    if !setup_succ {
        eprintln!("Failed to setup the benchmark");
        eprintln!("Exiting...");
    }
}

/// check if the core is available
fn core_available(core: u32) -> bool {
    let path = PathBuf::from(format!("/sys/devices/system/cpu/cpu{}/topology", core));
    if path.exists() && path.is_dir() {
        return true;
    }
    return false;
}

fn bind_core(core: u32) -> bool {
    unsafe {
        let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut cpuset);
        libc::CPU_SET(core as usize, &mut cpuset);

        0 == libc::pthread_setaffinity_np(libc::pthread_self(), libc::CPU_SETSIZE as _, &mut cpuset)
    }
}

fn acpi_avaiable(core: u32) -> bool {
    let path = PathBuf::from(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_driver",
        core
    ));

    std::fs::read_to_string(path)
        .expect("Failed to open scaling_driver")
        .trim()
        .eq_ignore_ascii_case("acpi-cpufreq")
}

fn acpi_set_governor<S: AsRef<str>>(core: u32, new_governor: S) -> Option<String> {
    let old_governor = std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
        core
    ))
    .expect("Failed to open scaling_governor")
    .trim()
    .to_string();

    std::fs::write(
        format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
            core
        ),
        new_governor.as_ref(),
    )
    .expect("Failed to write scaling_governor");

    return Some(old_governor);
}

fn acpi_set_freq(core: u32, new_freq: u32) -> Option<u32> {
    let old_freq = match match std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_setspeed",
        core
    )) {
        Ok(freq) => freq,
        Err(_) => {
            eprintln!("Failed to open scaling_setspeed");
            return None;
        }
    }
    .trim()
    .parse::<u32>()
    {
        Ok(freq) => freq,
        Err(_) => {
            eprintln!("Failed to parse scaling_setspeed");
            return None;
        }
    };

    match std::fs::write(
        format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_setspeed",
            core
        ),
        new_freq.to_string(),
    ) {
        Ok(_) => return Some(old_freq),
        Err(_) => {
            eprintln!("Failed to write scaling_setspeed");
            return None;
        }
    }
}

fn cpufreq_available(core: u32, freq: u32) -> bool {
    let path = PathBuf::from(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_available_frequencies",
        core
    ));

    let raw_freqs =
        std::fs::read_to_string(path).expect("Failed to open scaling_available_frequencies");

    let freqs = raw_freqs
        .split(" ")
        .into_iter()
        .filter(|s| {
            if s.trim().is_empty() {
                return false;
            }
            return true;
        })
        .map(|s| match s.trim().parse::<u32>() {
            Ok(freq) => freq,
            Err(_) => {
                eprintln!(
                    "Failed to parse scaling_available_frequencies, InvalidDigit `{}`",
                    s
                );
                std::process::exit(1);
            }
        })
        .collect::<Vec<u32>>();

    for (i, available_freq) in freqs.iter().enumerate() {
        let available_freq = *available_freq;
        if available_freq == freq {
            if i == 0 {
                return true;
            }

            let prev_freq = freqs[i - 1];
            if available_freq - 1000 == prev_freq {
                eprintln!(
                    "The frequency {} is not available, using the closest one {}",
                    freq, prev_freq
                );
                return false;
            } else {
                return true;
            }
        }
    }

    eprintln!("The frequency {} is not available", freq);
    return false;
}

fn hr_sleep_bench(micro: time::Duration) -> Vec<u64> {
    let mut results = Vec::new();
    for _ in 0..NUM_ITERATIONS {
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        // hrsleep::hr_sleep(micro);
        std::thread::sleep(micro);
        let end = unsafe { core::arch::x86_64::_rdtsc() };
        results.push(end - start);
    }
    results.sort();
    results
}

fn nano_sleep_bench(micro: time::Duration) -> Vec<u64> {
    let mut results = Vec::new();
    for _ in 0..NUM_ITERATIONS {
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        std::thread::sleep(micro);
        // hrsleep::nanosleep(micro);
        let end = unsafe { core::arch::x86_64::_rdtsc() };
        results.push(end - start);
    }
    results.sort();
    results
}
