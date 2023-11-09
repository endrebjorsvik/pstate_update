use std::fs;
use std::path;
use std::process;
use std::str::FromStr;
use std::{fmt, io};

/// Power profile exposed by power-profiles-daemon (PPD)
enum PPDPowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl FromStr for PPDPowerProfile {
    type Err = String;
    fn from_str(input: &str) -> Result<PPDPowerProfile, Self::Err> {
        match input {
            "power-saver" => Ok(PPDPowerProfile::PowerSaver),
            "balanced" => Ok(PPDPowerProfile::Balanced),
            "performance" => Ok(PPDPowerProfile::Performance),
            _ => Err(format!("Could not parse {input}")),
        }
    }
}

impl fmt::Display for PPDPowerProfile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PPDPowerProfile::PowerSaver => write!(f, "power-saver"),
            PPDPowerProfile::Balanced => write!(f, "balanced"),
            PPDPowerProfile::Performance => write!(f, "performance"),
        }
    }
}

impl PPDPowerProfile {
    /// Convert a PowerProfile to a matching EPP.
    fn to_epp(&self) -> EnergyPerformancePreference {
        match self {
            PPDPowerProfile::PowerSaver => EnergyPerformancePreference::Power,
            PPDPowerProfile::Balanced => EnergyPerformancePreference::BalancePower, // Or BalancePerformance?
            PPDPowerProfile::Performance => EnergyPerformancePreference::Performance,
        }
    }
}

/// Energy Performance Preference (EPP) exposed by the AMD PState driver
enum EnergyPerformancePreference {
    #[allow(dead_code)]
    Default,
    Performance,
    #[allow(dead_code)]
    BalancePerformance,
    BalancePower,
    Power,
}

impl fmt::Display for EnergyPerformancePreference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EnergyPerformancePreference::Default => write!(f, "default"),
            EnergyPerformancePreference::Performance => write!(f, "performance"),
            EnergyPerformancePreference::BalancePerformance => write!(f, "balance_performance"),
            EnergyPerformancePreference::BalancePower => write!(f, "balance_power"),
            EnergyPerformancePreference::Power => write!(f, "power"),
        }
    }
}

/// EPPController controls the CPU EPP levels
struct EPPController {
    // bus_name: String,
    // object_path: String,
    core_files: Vec<path::PathBuf>,
}

#[zbus::dbus_proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfilesDaemonManager {
    #[dbus_proxy(property)]
    fn active_profile(&self) -> zbus::Result<String>;
}

impl EPPController {
    /// Write the provided EPP to the CPU core given by the file path.
    fn write_epp_to_core(
        epp: &EnergyPerformancePreference,
        epp_file: &path::Path,
    ) -> io::Result<()> {
        log::debug!("Writing EPP '{epp}' to file {epp_file:?}.");
        fs::write(epp_file, epp.to_string())?;
        Ok(())
    }

    /// Write the provided EPP to all discovered CPU cores.
    fn write_epp_to_all_cores(&self, epp: &EnergyPerformancePreference) -> io::Result<()> {
        log::info!("Writing EPP {epp} to all EPP files.");
        for f in self.core_files.iter() {
            if let Err(e) = EPPController::write_epp_to_core(epp, f) {
                log::error!("Failed to write EPP to core ({f:?}): {e}.")
            }
        }
        Ok(())
    }

    /// Listen for PowerProfiles property changes on DBus and act on relvant changes.
    fn run(&self) -> Result<(), zbus::Error> {
        let conn = zbus::blocking::Connection::system()?;
        let proxy = PowerProfilesDaemonManagerProxyBlocking::new(&conn)?;
        let active = proxy.active_profile()?;
        // The general strategy is to fail early here, but not fail on later property changes.
        // If we encounter errors on property changes, they will mainly be logged.
        self.process_active_profile_changed(&active)?;

        let mut active_profile_changes = proxy.receive_active_profile_changed();
        log::info!(
            "Starting to listen for ActiveProfile changes on {}, {}.",
            proxy.destination(),
            proxy.path(),
        );
        for change in &mut active_profile_changes {
            let val = change.get()?;
            if let Err(e) = self.process_active_profile_changed(&val) {
                log::error!("Failed to process ActiveProfile change ({val}): {e}.");
            }
        }
        log::info!("Finished listening for property changes.");
        Ok(())
    }

    /// Process the provided property change value and write EPPs from it.
    fn process_active_profile_changed(&self, value: &str) -> Result<(), zbus::Error> {
        let profile = match PPDPowerProfile::from_str(value) {
            Ok(p) => p,
            Err(e) => {
                return Err(zbus::Error::Failure(e));
            }
        };
        log::info!("ActiveProfile changed: {profile}");
        self.write_epp_to_all_cores(&profile.to_epp())?;
        Ok(())
    }
}

/// Traverse the given cpufreq folder and collect valid EPP files for each CPU core
fn find_cpu_core_epp_paths(cpufreq_path: &path::Path) -> Result<Vec<path::PathBuf>, io::Error> {
    let mut paths = Vec::new();
    log::info!("Looking for EPP files for individual CPU cores in {cpufreq_path:?}.");
    for entry in cpufreq_path.read_dir()? {
        let p = entry.expect("any path from read_dir should be Ok").path();
        let dirname = match p.file_name() {
            Some(f) => match f.to_str() {
                Some(s) => s,
                None => continue,
            },
            None => continue,
        };
        if !dirname.starts_with("policy") {
            continue;
        }
        let epp_file = p.join("energy_performance_preference");
        if !epp_file.exists() {
            log::warn!("EPP file does not exist: {epp_file:?}.");
            continue;
        }
        log::debug!("Found valid EPP file: {epp_file:?}.");
        paths.push(epp_file);
    }
    log::info!("Found {} valid EPP files.", paths.len());
    Ok(paths)
}

fn main() {
    let env = env_logger::Env::new().default_filter_or("info");
    env_logger::init_from_env(env);

    let cpufreq_path = path::Path::new("/sys/devices/system/cpu/cpufreq");
    let epp_files = match find_cpu_core_epp_paths(cpufreq_path) {
        Ok(v) => v,
        Err(e) => {
            log::error!("{e}");
            process::exit(1);
        }
    };
    if epp_files.is_empty() {
        log::error!("Could not find any valid EPP files. Exiting.");
        process::exit(1);
    }

    let controller = EPPController {
        core_files: epp_files,
    };
    loop {
        match controller.run() {
            Ok(()) => {
                log::info!("Controller finished without error. Respawning.")
            }
            Err(e) => {
                log::error!("Encountered error. Exiting. {e}");
                process::exit(1);
            }
        }
    }
}
