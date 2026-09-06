//! Host-level sampling: CPU, memory and free disk space.

use std::path::Path;

use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System,
};

use crate::model::{DiskSpace, SystemStats};

const CPU_SENSOR_LABELS: [&str; 5] = ["tctl", "tdie", "package id 0", "tccd1", "core 0"];

pub struct SystemSampler {
    system: System,
    disks: Disks,
    components: Components,
    pid: Pid,
    temp_sensor: Option<usize>,
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
            temp_sensor: None,
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

        if self.temp_sensor.is_none() {
            self.temp_sensor = Some(self.find_temp_sensor()?);
        }

        let index = self.temp_sensor?;
        self.components.list().get(index)?.temperature()
    }

    /// Picks the sensor to read, most specific label first.
    fn find_temp_sensor(&self) -> Option<usize> {
        let labels: Vec<(usize, String)> = self
            .components
            .list()
            .iter()
            .enumerate()
            .filter(|(_, component)| component.temperature().is_some())
            .map(|(index, component)| (index, component.label().to_lowercase()))
            .collect();

        let find = |matches: &dyn Fn(&str) -> bool| {
            labels
                .iter()
                .find(|(_, label)| matches(label))
                .map(|(index, _)| *index)
        };

        CPU_SENSOR_LABELS
            .iter()
            .find_map(|wanted| find(&|label| label.contains(wanted)))
            .or_else(|| find(&|label| label.contains("cpu") || label.contains("k10temp")))
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
            // `symlink_metadata`, not `metadata`: a symlinked directory would otherwise be walked,
            // and a loop would never terminate.
            let Ok(metadata) = entry.path().symlink_metadata() else {
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
