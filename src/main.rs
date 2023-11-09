use std::fs;
use std::path;
use std::process;
use std::str::FromStr;
use std::{fmt, io};

use zbus::zvariant::Value;

/// Power profile exposed by power-profiles-daemon (PPD)
enum PPDPowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl FromStr for PPDPowerProfile {
    type Err = ();
    fn from_str(input: &str) -> Result<PPDPowerProfile, Self::Err> {
        match input {
            "power-saver" => Ok(PPDPowerProfile::PowerSaver),
            "balanced" => Ok(PPDPowerProfile::Balanced),
            "performance" => Ok(PPDPowerProfile::Performance),
            _ => Err(()),
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
    bus_name: String,
    object_path: String,
    property_name: String,
    core_files: Vec<path::PathBuf>,
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
        let proxy = zbus::blocking::fdo::PropertiesProxy::builder(&conn)
            .destination(self.bus_name.as_str())?
            .path(self.object_path.as_str())?
            .build()?;
        let mut changes = proxy.receive_properties_changed()?;

        // TODO: Ping that service is available.
        log::info!(
            "Starting to listen for property changes on {}, {}.",
            self.bus_name,
            self.object_path
        );
        while let Some(change) = changes.next() {
            self.process_property_change(change)?;
        }
        log::info!("Finished listening for property changes.");
        Ok(())
    }

    /// Process the provided property change and write EPPs from it.
    fn process_property_change(
        &self,
        change: zbus::blocking::fdo::PropertiesChanged,
    ) -> Result<(), zbus::Error> {
        let args = change.args()?;
        for (name, value) in args.changed_properties().iter() {
            if *name != self.property_name {
                log::info!("Ignoring property: {name}");
                continue;
            }
            let prop = match value {
                Value::Str(s) => s,
                v => {
                    log::error!("Unexpected property value type: {v:?}");
                    continue;
                }
            };
            let profile = match PPDPowerProfile::from_str(prop) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Could not parse property value '{prop}': {e:?}.");
                    continue;
                }
            };
            log::info!("Changed {name}: {profile}");
            self.write_epp_to_all_cores(&profile.to_epp())?
        }
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
        log::info!("Found valid EPP file: {epp_file:?}.");
        paths.push(epp_file);
    }
    Ok(paths)
}

fn main() {
    // TODO: Reswpawn if returned Ok. Just means that the service was restarted.
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
    if epp_files.len() == 0 {
        log::error!("Could not find any valid EPP files. Exiting.");
        process::exit(1);
    }
    let bus_name = String::from("net.hadess.PowerProfiles");
    let object_path = String::from("/net/hadess/PowerProfiles");
    let property_name = String::from("ActiveProfile");

    let controller = EPPController {
        bus_name: bus_name,
        object_path: object_path,
        property_name: property_name,
        core_files: epp_files,
    };
    match controller.run() {
        Ok(()) => log::info!("Success!"),
        Err(e) => {
            log::error!("Encountered error. Exiting. {e}");
            process::exit(1);
        }
    }
}
