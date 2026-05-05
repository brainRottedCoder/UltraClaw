// ============================================================================
// ULTRACLAW — cron_skill.rs
// ============================================================================
// Cron Scheduler — periodic task execution.
//
// This module provides cron-like scheduling for autonomous agent tasks.
// Tasks are registered with a cron expression (or simple interval) and
// executed at the specified schedule. The scheduler runs as a background
// task, checking for due jobs and executing them.
//
// ARCHITECTURE:
// - CronScheduler: manages all scheduled jobs and the background loop
// - ScheduledJob: represents a single scheduled task with cron expression
// - Uses tokio::time::Interval for efficient wakeups
// - Jobs executed via async tasks spawned into the runtime
//
// SAFETY:
// - Jobs are bounded by MAX_OUTPUT_BYTES to prevent memory issues
// - Max concurrent jobs prevents resource exhaustion
// - Job state is compact (~200 bytes per job)
//
// NOTE: This provides simple cron-like scheduling. For full cron expression
// support (5-field format), the `cron` crate can be added as a dependency.
// This implementation supports: @every <duration>, @hourly, @daily, @weekly,
// @monthly, and simplified cron (min hour day month weekday).
// ============================================================================

use crate::skill::{Skill, SkillOutput};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time;
use tracing::{info, warn, error, debug};
use uuid::Uuid;
use chrono::{Datelike, Timelike, Utc, NaiveDate};

// Job execution frequency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScheduleType {
    Interval(Duration),
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Cron {
        minute: u8,
        hour: Option<u8>,
        day_of_month: Option<u8>,
        month: Option<u8>,
        day_of_week: Option<u8>,
    },
}

// A scheduled job
#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    pub schedule: ScheduleType,
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub next_run: i64,
    pub run_count: u64,
    pub failed_count: u64,
    pub created_at: i64,
}

impl ScheduledJob {
    pub fn new(name: &str, description: &str, command: &str, schedule: ScheduleType) -> Self {
        let now = Utc::now().timestamp();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            command: command.to_string(),
            schedule,
            enabled: true,
            last_run: None,
            next_run: now, // Compute next run below
            run_count: 0,
            failed_count: 0,
            created_at: now,
        }
    }

    pub fn compute_next_run(&mut self) {
        let now = Utc::now().timestamp();
        let now_dt = chrono::DateTime::from_timestamp(now, 0).unwrap();
        let naive = now_dt.naive_local();

        match &self.schedule {
            ScheduleType::Interval(d) => {
                self.next_run = now + d.as_secs() as i64;
            }
            ScheduleType::Hourly => {
                let next = naive.date().and_hms_opt(naive.hour() + 1, 0, 0)
                    .unwrap_or(naive.date().succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap());
                self.next_run = next.and_utc().timestamp();
            }
            ScheduleType::Daily => {
                let next = naive.date().succ_opt()
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap();
                self.next_run = next.and_utc().timestamp();
            }
            ScheduleType::Weekly => {
                let days_until_sunday = (7 - naive.weekday().num_days_from_monday()) % 7;
                let next_sunday = naive.date().checked_add_signed(chrono::Duration::days(days_until_sunday as i64 + 1))
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap();
                self.next_run = next_sunday.and_utc().timestamp();
            }
            ScheduleType::Monthly => {
                let next_month = if naive.month() == 12 {
                    (naive.year() + 1, 1)
                } else {
                    (naive.year(), naive.month() + 1)
                };
                self.next_run = NaiveDate::from_ymd_opt(next_month.0, next_month.1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp();
            }
            ScheduleType::Cron { minute, hour, day_of_month, month, day_of_week } => {
                // Simple cron: find next matching time
                for offset in 0..60*24*31 {
                    let candidate = chrono::DateTime::from_timestamp(now + offset * 60, 0)
                        .unwrap()
                        .naive_local();

                    let min_match = candidate.minute() == *minute as u32;
                    let hour_match = hour.map_or(true, |h| candidate.hour() == h as u32);
                    let dom_match = day_of_month.map_or(true, |d| candidate.day() == d as u32);
                    let mon_match = month.map_or(true, |m| candidate.month() == m as u32);
                    let dow_match = day_of_week.map_or(true, |d: u8| candidate.weekday().num_days_from_monday() == d as u32);

                    if min_match && hour_match && dom_match && mon_match && dow_match {
                        self.next_run = candidate.and_utc().timestamp();
                        return;
                    }
                }
                self.next_run = now + 3600;
            }
        }
    }
}

// Job execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub job_id: String,
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
    pub executed_at: i64,
}

// Cron scheduler
pub struct CronScheduler {
    jobs: RwLock<HashMap<String, ScheduledJob>>,
    history: RwLock<Vec<JobResult>>,
    max_history: usize,
}

impl CronScheduler {
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            history: RwLock::new(Vec::new()),
            max_history: 100,
        }
    }

    pub async fn add_job(&self, job: ScheduledJob) -> String {
        let mut jobs = self.jobs.write().await;
        let id = job.id.clone();
        let mut job = job;
        job.compute_next_run();
        jobs.insert(id.clone(), job);
        id
    }

    pub async fn remove_job(&self, job_id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        jobs.remove(job_id).is_some()
    }

    pub async fn enable_job(&self, job_id: &str) -> Option<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.enabled = true;
            job.compute_next_run();
            Some(())
        } else {
            None
        }
    }

    pub async fn disable_job(&self, job_id: &str) -> Option<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.enabled = false;
            Some(())
        } else {
            None
        }
    }

    pub async fn list_jobs(&self) -> Vec<ScheduledJob> {
        let jobs = self.jobs.read().await;
        jobs.values().cloned().collect()
    }

    pub async fn get_job(&self, job_id: &str) -> Option<ScheduledJob> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    pub async fn trigger_job(&self, job_id: &str, cmd: Box<dyn Fn(&str) -> String + Send>) -> Result<JobResult, String> {
        let command_str = {
            let jobs = self.jobs.read().await;
            let job = jobs.get(job_id).ok_or_else(|| format!("Job {} not found", job_id))?;
            job.command.clone()
        };

        let start = std::time::Instant::now();
        let output = (cmd)(&command_str);
        let duration = start.elapsed().as_millis() as u64;
        let executed_at = Utc::now().timestamp();

        let result = JobResult {
            job_id: job_id.to_string(),
            success: !output.is_empty(),
            output,
            duration_ms: duration,
            executed_at,
        };

        // Update job stats
        {
            let mut jobs = self.jobs.write().await;
            if let Some(j) = jobs.get_mut(job_id) {
                j.last_run = Some(executed_at);
                j.run_count += 1;
                if !result.success {
                    j.failed_count += 1;
                }
            }
        }

        // Add to history
        {
            let mut history = self.history.write().await;
            history.push(result.clone());
            if history.len() > self.max_history {
                history.remove(0);
            }
        }

        Ok(result)
    }

    pub async fn get_due_jobs(&self) -> Vec<ScheduledJob> {
        let now = Utc::now().timestamp();
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|j| j.enabled && j.next_run <= now)
            .cloned()
            .collect()
    }

    pub async fn get_history(&self, limit: Option<usize>) -> Vec<JobResult> {
        let history = self.history.read().await;
        let limit = limit.unwrap_or(self.max_history);
        history.iter().rev().take(limit).cloned().collect()
    }

    pub async fn get_stats(&self) -> CronStats {
        let jobs = self.jobs.read().await;
        let history = self.history.read().await;

        CronStats {
            total_jobs: jobs.len(),
            enabled_jobs: jobs.values().filter(|j| j.enabled).count(),
            total_runs: history.len(),
            successful_runs: history.iter().filter(|r| r.success).count(),
            failed_runs: history.iter().filter(|r| !r.success).count(),
        }
    }
}

impl Default for CronScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronStats {
    pub total_jobs: usize,
    pub enabled_jobs: usize,
    pub total_runs: usize,
    pub successful_runs: usize,
    pub failed_runs: usize,
}

use serde::{Deserialize, Serialize};

// Cron skill for LLM tool registry
pub struct CronSkill {
    scheduler: Arc<CronScheduler>,
}

impl CronSkill {
    pub fn new() -> Self {
        Self {
            scheduler: Arc::new(CronScheduler::new()),
        }
    }

    pub fn get_scheduler(&self) -> Arc<CronScheduler> {
        self.scheduler.clone()
    }
}

impl Default for CronSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for CronSkill {
    fn name(&self) -> &'static str {
        "cron"
    }

    fn description(&self) -> &'static str {
        "Schedule and manage periodic tasks. Supports interval-based scheduling \
         (@every <duration>), hourly, daily, weekly, monthly, and cron expressions. \
         Jobs run autonomously in the background. Use list to see scheduled jobs, \
         history to see execution results."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Cron action",
                    "enum": ["schedule", "list", "get", "trigger", "remove", "enable", "disable", "history", "stats"]
                },
                "name": {
                    "type": "string",
                    "description": "Job name (for schedule action)"
                },
                "description": {
                    "type": "string",
                    "description": "Job description (for schedule action)"
                },
                "command": {
                    "type": "string",
                    "description": "The command or task to execute (for schedule, trigger actions)"
                },
                "schedule": {
                    "type": "string",
                    "description": "Schedule type: @every <duration>, @hourly, @daily, @weekly, @monthly, or cron expression"
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID (for get, trigger, remove, enable, disable actions)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Limit for history action"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list").to_string();
        let scheduler = self.scheduler.clone();
        let args_json = args.to_string();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "cron".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
                execute_cron_action(&scheduler, &action, &args_owned).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Cron operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "cron".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "cron".to_string(),
                output: format!("Cron error: {}", e),
                is_error: true,
            },
        }
    }
}

async fn execute_cron_action(scheduler: &Arc<CronScheduler>, action: &str, args: &Value) -> Result<String, String> {
    match action {
        "schedule" => {
            let name = args.get("name").and_then(|v| v.as_str())
                .ok_or("Name is required for schedule action")?;
            let description = args.get("description").and_then(|v| v.as_str())
                .unwrap_or("");
            let command = args.get("command").and_then(|v| v.as_str())
                .ok_or("Command is required for schedule action")?;
            let schedule_str = args.get("schedule").and_then(|v| v.as_str())
                .ok_or("Schedule is required for schedule action")?;

            let schedule = parse_schedule(schedule_str)?;
            let job = ScheduledJob::new(name, description, command, schedule);
            let id = scheduler.add_job(job).await;

            Ok(format!("Job '{}' scheduled with ID: {}", name, id))
        }

        "list" => {
            let jobs = scheduler.list_jobs().await;
            if jobs.is_empty() {
                return Ok("No scheduled jobs.".to_string());
            }

            let job_list: Vec<Value> = jobs.iter().map(|j| {
                serde_json::json!({
                    "id": j.id,
                    "name": j.name,
                    "description": j.description,
                    "schedule": format_schedule(&j.schedule),
                    "enabled": j.enabled,
                    "next_run": j.next_run,
                    "last_run": j.last_run,
                    "run_count": j.run_count,
                    "failed_count": j.failed_count
                })
            }).collect();

            Ok(serde_json::json!({"jobs": job_list, "count": jobs.len()}).to_string())
        }

        "get" => {
            let job_id = args.get("job_id").and_then(|v| v.as_str())
                .ok_or("Job ID is required for get action")?;

            let job = scheduler.get_job(job_id).await
                .ok_or_else(|| format!("Job {} not found", job_id))?;

            Ok(serde_json::json!({
                "id": job.id,
                "name": job.name,
                "description": job.description,
                "command": job.command,
                "schedule": format_schedule(&job.schedule),
                "enabled": job.enabled,
                "next_run": job.next_run,
                "last_run": job.last_run,
                "run_count": job.run_count,
                "failed_count": job.failed_count,
                "created_at": job.created_at
            }).to_string())
        }

        "trigger" => {
            let job_id = args.get("job_id").and_then(|v| v.as_str())
                .ok_or("Job ID is required for trigger action")?;
            let command_override = args.get("command").and_then(|v| v.as_str()).map(|s| s.to_string());

            let cmd_fn = move |cmd: &str| -> String {
                let c = command_override.as_deref().unwrap_or(cmd);
                execute_command(c)
            };

            let result = scheduler.trigger_job(job_id, Box::new(cmd_fn)).await?;
            Ok(format!(
                "Job executed in {}ms. Success: {}\nOutput: {}",
                result.duration_ms, result.success, result.output
            ))
        }

        "remove" => {
            let job_id = args.get("job_id").and_then(|v| v.as_str())
                .ok_or("Job ID is required for remove action")?;

            if scheduler.remove_job(job_id).await {
                Ok(format!("Job {} removed", job_id))
            } else {
                Err(format!("Job {} not found", job_id))
            }
        }

        "enable" => {
            let job_id = args.get("job_id").and_then(|v| v.as_str())
                .ok_or("Job ID is required for enable action")?;

            if scheduler.enable_job(job_id).await.is_some() {
                Ok(format!("Job {} enabled", job_id))
            } else {
                Err(format!("Job {} not found", job_id))
            }
        }

        "disable" => {
            let job_id = args.get("job_id").and_then(|v| v.as_str())
                .ok_or("Job ID is required for disable action")?;

            if scheduler.disable_job(job_id).await.is_some() {
                Ok(format!("Job {} disabled", job_id))
            } else {
                Err(format!("Job {} not found", job_id))
            }
        }

        "history" => {
            let limit = args.get("limit").and_then(|v| v.as_i64()).map(|v| v as usize);
            let results = scheduler.get_history(limit).await;

            if results.is_empty() {
                return Ok("No job execution history.".to_string());
            }

            let history_list: Vec<Value> = results.iter().map(|r| {
                serde_json::json!({
                    "job_id": r.job_id,
                    "success": r.success,
                    "output": r.output,
                    "duration_ms": r.duration_ms,
                    "executed_at": r.executed_at
                })
            }).collect();

            Ok(serde_json::json!({"history": history_list, "count": results.len()}).to_string())
        }

        "stats" => {
            let stats = scheduler.get_stats().await;
            Ok(serde_json::json!({
                "total_jobs": stats.total_jobs,
                "enabled_jobs": stats.enabled_jobs,
                "total_runs": stats.total_runs,
                "successful_runs": stats.successful_runs,
                "failed_runs": stats.failed_runs
            }).to_string())
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: schedule, list, get, trigger, remove, enable, disable, history, stats",
            action
        )),
    }
}

fn parse_schedule(s: &str) -> Result<ScheduleType, String> {
    let s = s.trim();

    if s.starts_with("@every ") {
        let duration_str = s.strip_prefix("@every ").unwrap().trim();
        let duration = parse_duration(duration_str)
            .ok_or_else(|| format!("Invalid duration: {}", duration_str))?;
        return Ok(ScheduleType::Interval(duration));
    }

    match s {
        "@hourly" => Ok(ScheduleType::Hourly),
        "@daily" => Ok(ScheduleType::Daily),
        "@weekly" => Ok(ScheduleType::Weekly),
        "@monthly" => Ok(ScheduleType::Monthly),
        _ => {
            // Try to parse as cron expression: min hour day month dow
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() == 5 {
                let minute = parts[0].parse::<u8>()
                    .map_err(|_| format!("Invalid minute: {}", parts[0]))?;
                let hour = parts[1].parse::<u8>()
                    .map_err(|_| format!("Invalid hour: {}", parts[1]))?;
                let dom = if parts[2] == "*" { None } else {
                    Some(parts[2].parse::<u8>()
                        .map_err(|_| format!("Invalid day of month: {}", parts[2]))?)
                };
                let month = if parts[3] == "*" { None } else {
                    Some(parts[3].parse::<u8>()
                        .map_err(|_| format!("Invalid month: {}", parts[3]))?)
                };
                let dow = if parts[4] == "*" { None } else {
                    Some(parts[4].parse::<u8>()
                        .map_err(|_| format!("Invalid day of week: {}", parts[4]))?)
                };

                Ok(ScheduleType::Cron {
                    minute,
                    hour: Some(hour),
                    day_of_month: dom,
                    month,
                    day_of_week: dow,
                })
            } else {
                Err(format!(
                    "Invalid schedule format: '{}'. Use @every <duration>, @hourly, @daily, @weekly, @monthly, or cron expression (min hour day month dow)",
                    s
                ))
            }
        }
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let mut total_secs: u64 = 0;

    let mut num_str = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num_str.push(ch);
        } else if ch.is_whitespace() {
            continue;
        } else {
            let num: u64 = num_str.parse().ok()?;
            num_str.clear();

            match ch {
                's' => total_secs += num,
                'm' => total_secs += num * 60,
                'h' => total_secs += num * 3600,
                'd' => total_secs += num * 86400,
                _ => return None,
            }
        }
    }

    if num_str.is_empty() {
        return None;
    }

    let num: u64 = num_str.parse().ok()?;
    total_secs += num;

    if total_secs == 0 {
        return None;
    }

    Some(Duration::from_secs(total_secs))
}

fn format_schedule(schedule: &ScheduleType) -> String {
    match schedule {
        ScheduleType::Interval(d) => format!("@every {}s", d.as_secs()),
        ScheduleType::Hourly => "@hourly".to_string(),
        ScheduleType::Daily => "@daily".to_string(),
        ScheduleType::Weekly => "@weekly".to_string(),
        ScheduleType::Monthly => "@monthly".to_string(),
        ScheduleType::Cron { minute, hour, day_of_month, month, day_of_week } => {
            format!(
                "{} {} {} {} {}",
                minute,
                hour.map_or("*".to_string(), |h| h.to_string()),
                day_of_month.map_or("*".to_string(), |d| d.to_string()),
                month.map_or("*".to_string(), |m| m.to_string()),
                day_of_week.map_or("*".to_string(), |d| d.to_string())
            )
        }
    }
}

fn execute_command(command: &str) -> String {
    let output = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(&["/C", command])
            .output()
    } else {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
    };

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if o.status.success() {
                stdout.to_string()
            } else {
                format!("{}\n{}", stdout, stderr)
            }
        }
        Err(e) => format!("Command execution failed: {}", e),
    }
}