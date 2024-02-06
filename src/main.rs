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

#[derive(serde::Deserialize)]
/// Energy Performance Preference (EPP) exposed by the AMD P-State driver
enum EnergyPerformancePreference {
    #[allow(dead_code)]
    #[serde(rename(deserialize = "default"))]
    Default,
    #[serde(rename(deserialize = "performance"))]
    Performance,
    #[allow(dead_code)]
    #[serde(rename(deserialize = "balance_performance"))]
    BalancePerformance,
    #[serde(rename(deserialize = "balance_power"))]
    BalancePower,
    #[serde(rename(deserialize = "power"))]
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

impl FromStr for EnergyPerformancePreference {
    type Err = String;
    fn from_str(input: &str) -> Result<EnergyPerformancePreference, Self::Err> {
        match input {
            "default" => Ok(EnergyPerformancePreference::Default),
            "performance" => Ok(EnergyPerformancePreference::Performance),
            "balance_performance" => Ok(EnergyPerformancePreference::BalancePerformance),
            "balance_power" => Ok(EnergyPerformancePreference::BalancePower),
            "power" => Ok(EnergyPerformancePreference::Power),
            _ => Err(format!("Could not parse {input}")),
        }
    }
}

/// Scaling governor exposed by the AMD P-State driver
#[derive(serde::Deserialize)]
enum ScalingGovernor {
    #[serde(rename(deserialize = "powersave"))]
    PowerSave,
    #[serde(rename(deserialize = "performance"))]
    Performance,
}

impl fmt::Display for ScalingGovernor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ScalingGovernor::Performance => write!(f, "performance"),
            ScalingGovernor::PowerSave => write!(f, "powersave"),
        }
    }
}

impl FromStr for ScalingGovernor {
    type Err = String;
    fn from_str(input: &str) -> Result<ScalingGovernor, Self::Err> {
        match input {
            "powersave" => Ok(ScalingGovernor::PowerSave),
            "performance" => Ok(ScalingGovernor::Performance),
            _ => Err(format!("Could not parse {input}")),
        }
    }
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

/// `EPPController` controls the CPU EPP levels
struct EPPController {
    epp_core_files: Vec<path::PathBuf>,
    epp_config: EPPConfig,
    governor_core_files: Vec<path::PathBuf>,
    governor_config: GovernorConfig,
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
    fn write_epp_to_all_cores(&self, epp: &EnergyPerformancePreference) {
        log::info!("Writing EPP {epp} to all EPP files.");
        for f in &self.epp_core_files {
            if let Err(e) = EPPController::write_epp_to_core(epp, f) {
                log::error!("Failed to write EPP to core ({f:?}): {e}.");
            }
        }
    }

    /// Write the provided scaling governor to the CPU core given by the file path.
    fn write_governor_to_core(gov: &ScalingGovernor, gov_file: &path::Path) -> io::Result<()> {
        log::debug!("Writing governor '{gov}' to file {gov_file:?}.");
        fs::write(gov_file, gov.to_string())?;
        Ok(())
    }

    /// Write the provided EPP to all discovered CPU cores.
    fn write_governor_to_all_cores(&self, gov: &ScalingGovernor) {
        log::info!("Writing governor {gov} to all governor files.");
        for f in &self.governor_core_files {
            if let Err(e) = EPPController::write_governor_to_core(gov, f) {
                log::error!("Failed to write governor to core ({f:?}): {e}.");
            }
        }
    }

    /// Listen for `PowerProfiles` property changes on D-Bus and act on relvant changes.
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
        self.write_governor_to_all_cores(self.desired_governor(&profile));
        self.write_epp_to_all_cores(self.desired_epp(&profile));
        Ok(())
    }

    /// Select appropriate EPP from Power profile.
    fn desired_epp(&self, profile: &PPDPowerProfile) -> &EnergyPerformancePreference {
        match profile {
            PPDPowerProfile::Performance => &self.epp_config.performance,
            PPDPowerProfile::Balanced => &self.epp_config.balanced,
            PPDPowerProfile::PowerSaver => &self.epp_config.power_saver,
        }
    }

    /// Select appropriate Scaling Governor from Power profile.
    fn desired_governor(&self, profile: &PPDPowerProfile) -> &ScalingGovernor {
        match profile {
            PPDPowerProfile::Performance => &self.governor_config.performance,
            PPDPowerProfile::Balanced => &self.governor_config.balanced,
            PPDPowerProfile::PowerSaver => &self.governor_config.power_saver,
        }
    }
}

/// Traverse the given `cpufreq` folder and collect valid EPP files for each CPU core
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

fn generate_cpu_core_gorvernor_paths(epp_paths: &[path::PathBuf]) -> Vec<path::PathBuf> {
    let mut paths = Vec::new();
    for epp in epp_paths {
        let p = epp.with_file_name("scaling_governor");
        if !p.exists() {
            log::warn!("Governor file does not exist: {p:?}.");
            continue;
        }
        paths.push(p);
    }
    log::info!("Found {} valid governor files.", paths.len());
    paths
}

#[derive(serde::Deserialize)]
struct EPPConfig {
    power_saver: EnergyPerformancePreference,
    balanced: EnergyPerformancePreference,
    performance: EnergyPerformancePreference,
}

#[derive(serde::Deserialize)]
struct GovernorConfig {
    power_saver: ScalingGovernor,
    balanced: ScalingGovernor,
    performance: ScalingGovernor,
}

#[derive(serde::Deserialize)]
struct Config {
    epp: EPPConfig,
    scaling_governor: GovernorConfig,
}

fn read_config() -> Result<Config, io::Error> {
    let mut config_file = path::Path::new("/etc/pstate_update/config.toml");
    if !config_file.exists() {
        log::warn!("Could not find {config_file:?}. Trying local folder instead.");
        config_file = path::Path::new("config.toml");
    }
    let s = fs::read_to_string(config_file)?;
    let config: Config = match toml::from_str(&s) {
        Ok(c) => c,
        Err(e) => {
            return Err(io::Error::new(io::ErrorKind::Other, e));
        }
    };
    Ok(config)
}

fn main() {
    // TODO: Notify desktop on certain errors? Could be easily done using DBus.
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
    let governor_files = generate_cpu_core_gorvernor_paths(&epp_files);
    if epp_files.is_empty() {
        log::error!("Could not find any valid governor files. Exiting.");
        process::exit(1);
    }
    let config = match read_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e}");
            process::exit(1);
        }
    };

    let controller = EPPController {
        epp_core_files: epp_files,
        epp_config: config.epp,
        governor_core_files: governor_files,
        governor_config: config.scaling_governor,
    };
    loop {
        match controller.run() {
            Ok(()) => {
                log::info!("Controller finished without error. Respawning.");
            }
            Err(e) => {
                log::error!("Encountered error. Exiting. {e}");
                process::exit(1);
            }
        }
    }
}
