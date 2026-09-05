//! Host-level sampling: CPU, memory and free disk space.

use std::path::Path;

use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System,
};

/// What one host sample yields.
#[derive(Clone, Debug, Default)]
pub struct SystemStats {
    /// Total CPU usage in percent, `0.0..=100.0`.
    pub cpu_total: f32,
    /// Per-core usage in percent, in the order the OS reports cores.
    pub cpu_per_core: Vec<f32>,
    /// Resident set size of this process.
    pub mem_process_rss: u64,
    pub mem_system_used: u64,
    pub mem_system_total: u64,
    /// CPU package temperature in degrees Celsius, if the machine exposes one.
    pub cpu_temp_c: Option<f32>,
}

const CPU_SENSOR_LABELS: [&str; 5] = ["tctl", "tdie", "package id 0", "tccd1", "core 0"];

/// Free and total bytes of the filesystem a path lives on.
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskSpace {
    pub free: u64,
    pub total: u64,
}

pub struct SystemSampler {
    system: System,
    disks: Disks,
    components: Components,
    pid: Pid,
}

impl SystemSampler {
    #[must_use]
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );

        Self {
            system,
            disks: Disks::new_with_refreshed_list(),
            components: Components::new_with_refreshed_list(),
            pid: Pid::from_u32(std::process::id()),
        }
    }

    /// Refreshes CPU and memory.
    pub fn sample(&mut self) -> SystemStats {
        self.system.refresh_cpu_usage();
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            false,
            ProcessRefreshKind::nothing().with_memory(),
        );

        SystemStats {
            cpu_total: self.system.global_cpu_usage(),
            cpu_per_core: self
                .system
                .cpus()
                .iter()
                .map(sysinfo::Cpu::cpu_usage)
                .collect(),
            mem_process_rss: self
                .system
                .process(self.pid)
                .map_or(0, sysinfo::Process::memory),
            mem_system_used: self.system.used_memory(),
            mem_system_total: self.system.total_memory(),
            cpu_temp_c: self.cpu_temperature(),
        }
    }

    /// The CPU package temperature, or `None` where no such sensor is exposed.
    fn cpu_temperature(&mut self) -> Option<f32> {
        self.components.refresh(false);

        let labelled: Vec<(String, f32)> = self
            .components
            .list()
            .iter()
            .filter_map(|component| {
                component
                    .temperature()
                    .map(|temp| (component.label().to_lowercase(), temp))
            })
            .collect();

        for wanted in CPU_SENSOR_LABELS {
            if let Some((_, temp)) = labelled.iter().find(|(label, _)| label.contains(wanted)) {
                return Some(*temp);
            }
        }

        labelled
            .iter()
            .find(|(label, _)| label.contains("cpu") || label.contains("k10temp"))
            .map(|(_, temp)| *temp)
    }

    /// Free and total space of the filesystem holding `path`.
    pub fn disk_space_for(&mut self, path: &Path) -> Option<DiskSpace> {
        self.disks.refresh(false);

        // Resolving symlinks matters: the configured world path is relative to the CWD.
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        self.disks
            .list()
            .iter()
            .filter(|disk| target.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len())
            .map(|disk| DiskSpace {
                free: disk.available_space(),
                total: disk.total_space(),
            })
    }
}

impl Default for SystemSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Total size of a directory tree, following no symlinks.
#[must_use]
pub fn directory_size(root: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![root.to_path_buf()];

    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total += metadata.len();
            }
        }
    }

    total
}
