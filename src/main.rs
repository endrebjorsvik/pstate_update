use std::fs;
use std::path;
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

fn set_epp_on_core(epp: &EnergyPerformancePreference, core_path: &path::Path) -> io::Result<()> {
    let f = core_path.join("energy_performance_preference");
    log::debug!("Writing EPP '{epp}' to file {f:?}.");
    fs::write(f, epp.to_string())?;
    Ok(())
}

fn set_epp_on_all_cores(epp: &EnergyPerformancePreference) -> io::Result<()> {
    let base = path::Path::new("/sys/devices/system/cpu/cpufreq");
    log::info!("Writing EPP {epp} to kernel under {base:?}.");
    for entry in base.read_dir()? {
        let p = entry.unwrap().path();
        let fname = match p.file_name() {
            Some(f) => match f.to_str() {
                Some(s) => s,
                None => continue,
            },
            None => continue,
        };
        if fname.starts_with("policy") {
            // TODO: Different error handling here?
            set_epp_on_core(epp, p.as_path())?
        }
    }
    Ok(())
}

fn run_listener() -> Result<(), zbus::Error> {
    let bus_name = "net.hadess.PowerProfiles";
    let object_path = "/net/hadess/PowerProfiles";
    let property_name = "ActiveProfile";
    // -> 'balanced'
    // Property interface: org.freedesktop.DBus.Properties
    // -> Method: (org.freedesktop.DBus.Properties.)PropertiesChanged

    let conn = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::fdo::PropertiesProxy::builder(&conn)
        .destination(bus_name)?
        .path(object_path)?
        .build()?;
    let mut changes = proxy.receive_properties_changed()?;
    // let mut changes = proxy.receive_property_changed(property_name);

    log::info!("Starting to listen fproperty_nameor property changes.");
    while let Some(change) = changes.next() {
        // To print the full message of `change`:
        // log::info!("Change body: {change:#?}");
        let args = change.args()?;
        for (name, value) in args.changed_properties().iter() {
            if *name != property_name {
                log::info!("Ignoring property: {name}");
                continue;
            }
            match value {
                Value::Str(s) => {
                    if let Ok(p) = PPDPowerProfile::from_str(s) {
                        log::info!("New {property_name}: {p}");
                        set_epp_on_all_cores(&p.to_epp())?
                    }
                }
                v => {
                    log::error!("ERROR: Unexpected profile value: {v:?}")
                }
            }
        }
    }
    log::info!("Finished listening for property changes.");

    Ok(())
}

fn main() {
    // TODO: Docstrings on all functions.
    // TODO: Reswpawn if returned Ok. Just means that the service was restarted.
    let env = env_logger::Env::new().default_filter_or("info");
    env_logger::init_from_env(env);
    match run_listener() {
        Ok(()) => log::info!("Success!"),
        Err(e) => log::error!("ERROR: {e}"),
    }
}
