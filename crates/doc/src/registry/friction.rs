//! Per-Space friction reports.
use super::*;
use mast_proto::{FrictionOccurrence, FrictionReport, FrictionStatus, derive_id};

pub const KIND_FRICTION_REPORTS: &str = "frictionReports";

/// Fingerprint independent of the originating chat and Space.
pub fn friction_fingerprint(report: &FrictionReport) -> String {
    let summary = report
        .summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let command = report
        .approval
        .as_ref()
        .map(|a| a.command.as_str())
        .unwrap_or("");
    let kind = serde_json::to_string(&report.kind).unwrap_or_default();
    derive_id(
        &format!("{kind}\0{summary}\0{command}"),
        "friction-fingerprint",
    )
}

const COUNTER_PREFIX: &str = "occurrence:";
const OCCURRENCES_KEPT: usize = 50;

fn decode(row: &RegistryRow) -> Option<FrictionReport> {
    let value = Value::Object(row.fields.clone().into_iter().collect());
    let mut report: FrictionReport = serde_json::from_value(value).ok()?;
    let counters: Vec<&Value> = row
        .fields
        .iter()
        .filter_map(|(name, value)| name.starts_with(COUNTER_PREFIX).then_some(value))
        .collect();
    if !counters.is_empty() {
        report.occurrences = counters
            .iter()
            .filter_map(|counter| counter.get("count").and_then(Value::as_u64))
            .fold(0, u64::saturating_add);
        report.occurrence_history = counters.iter()
            .filter_map(|counter| counter.get("history").and_then(Value::as_array))
            .flatten().filter_map(|value| serde_json::from_value::<FrictionOccurrence>(value.clone()).ok())
            .collect();
        report.occurrence_history.sort_by(|a, b| (a.at, &a.chat_id, &a.space_id).cmp(&(b.at, &b.chat_id, &b.space_id)));
        let excess = report.occurrence_history.len().saturating_sub(OCCURRENCES_KEPT);
        report.occurrence_history.drain(..excess);
        report.last_seen = counters
            .iter()
            .filter_map(|counter| counter.get("lastSeen").and_then(Value::as_i64))
            .max()
            .unwrap_or(report.last_seen);
    }
    Some(report)
}

impl RegistryDoc {
    pub fn friction_reports(&self, space_id: &str) -> Vec<FrictionReport> {
        self.overlay_rows(KIND_FRICTION_REPORTS)
            .iter()
            .filter_map(decode)
            .filter(|report| report.provenance.space_id == space_id)
            .collect()
    }

    /// Update the answer without recording another occurrence.
    pub fn answer_friction_approval(&mut self, id: &str, answer: bool) -> Result<(), DocError> {
        let row = self
            .overlay_row(KIND_FRICTION_REPORTS, id)
            .ok_or_else(|| DocError::Schema("friction report not found".into()))?;
        let mut report =
            decode(&row).ok_or_else(|| DocError::Schema("invalid friction report".into()))?;
        let approval = report
            .approval
            .as_mut()
            .ok_or_else(|| DocError::Schema("friction report has no approval".into()))?;
        approval.answer = Some(answer);
        let mut fields = BTreeMap::new();
        fields.insert("approval".into(), serde_json::to_value(approval)?);
        self.observe_row(KIND_FRICTION_REPORTS, id);
        let op = RowOp {
            kind: KIND_FRICTION_REPORTS.into(),
            id: id.into(),
            op: OpKind::Update,
            set: Some(fields),
            hlc: self.next_hlc(),
            clocks: None,
        };
        check_op_size(&op)?;
        self.write_ops(vec![op])?;
        Ok(())
    }

    pub fn set_friction_status(
        &mut self,
        space_id: &str,
        fingerprint: &str,
        status: FrictionStatus,
        promoted_item_id: Option<String>,
        promoted_at: Option<i64>,
    ) -> Result<FrictionReport, DocError> {
        let report = self
            .friction_reports(space_id)
            .into_iter()
            .find(|r| r.fingerprint == fingerprint)
            .ok_or_else(|| DocError::Schema("friction report not found".into()))?;
        let mut fields = BTreeMap::new();
        fields.insert("status".into(), serde_json::to_value(status)?);
        fields.insert(
            "promotedItemId".into(),
            serde_json::to_value(promoted_item_id)?,
        );
        fields.insert("promotedAt".into(), serde_json::to_value(promoted_at)?);
        self.observe_row(KIND_FRICTION_REPORTS, &report.id);
        let op = RowOp {
            kind: KIND_FRICTION_REPORTS.into(),
            id: report.id.clone(),
            op: OpKind::Update,
            set: Some(fields),
            hlc: self.next_hlc(),
            clocks: None,
        };
        check_op_size(&op)?;
        self.write_ops(vec![op])?;
        self.overlay_row(KIND_FRICTION_REPORTS, &report.id)
            .as_ref()
            .and_then(decode)
            .ok_or_else(|| DocError::Schema("friction report could not be read".into()))
    }

    /// Record an occurrence with a per-device counter, so concurrent devices merge.
    pub fn record_friction(&mut self, report: FrictionReport) -> Result<FrictionReport, DocError> {
        self.record_friction_occurrence(report, None)
            .map(|(report, _)| report)
    }

    /// Record one imported occurrence at most once per stable entry identifier.
    pub fn import_friction(
        &mut self,
        report: FrictionReport,
        import_id: &str,
    ) -> Result<(FrictionReport, bool), DocError> {
        let key = format!(
            "{COUNTER_PREFIX}{}",
            derive_id(import_id, "friction-import")
        );
        if let Some(existing) = self
            .overlay_rows(KIND_FRICTION_REPORTS)
            .iter()
            .filter(|row| row.fields.contains_key(&key))
            .filter_map(decode)
            .find(|row| row.provenance.space_id == report.provenance.space_id)
        {
            return Ok((existing, false));
        }
        self.record_friction_occurrence(report, Some(import_id))
    }

    fn record_friction_occurrence(
        &mut self,
        mut report: FrictionReport,
        import_id: Option<&str>,
    ) -> Result<(FrictionReport, bool), DocError> {
        if report.summary.trim().is_empty()
            || report.summary.chars().count() > 140
            || report.what_happened.trim().is_empty()
            || report.provenance.space_id.is_empty()
        {
            return Err(DocError::Schema(
                "friction requires a Space, summary of at most 140 characters, and what happened"
                    .into(),
            ));
        }
        report.fingerprint = friction_fingerprint(&report);
        report.id = derive_id(
            &format!("{}\0{}", report.provenance.space_id, report.fingerprint),
            "friction-report",
        );
        let state = self.row_state(KIND_FRICTION_REPORTS, &report.id);
        if state == RowState::Tombstoned {
            return Err(DocError::Schema("friction report was deleted".into()));
        }
        let existing = self.overlay_row(KIND_FRICTION_REPORTS, &report.id);
        let counter_key = match import_id {
            Some(id) => format!("{COUNTER_PREFIX}{}", derive_id(id, "friction-import")),
            None => format!(
                "{COUNTER_PREFIX}{}",
                derive_id(&self.device_id, "friction-device")
            ),
        };
        if import_id.is_some()
            && existing
                .as_ref()
                .is_some_and(|row| row.fields.contains_key(&counter_key))
        {
            return Ok((
                existing
                    .as_ref()
                    .and_then(decode)
                    .ok_or_else(|| DocError::Schema("invalid friction report".into()))?,
                false,
            ));
        }
        let counter = existing
            .as_ref()
            .and_then(|row| row.fields.get(&counter_key));
        let count = counter
            .and_then(|v| v.get("count"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .saturating_add(1);
        let last_seen = counter
            .and_then(|v| v.get("lastSeen"))
            .and_then(Value::as_i64)
            .unwrap_or(report.provenance.created_at)
            .max(report.provenance.created_at);
        let mut history: Vec<FrictionOccurrence> = counter
            .and_then(|v| v.get("history"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        history.push(FrictionOccurrence { at: report.provenance.created_at,
            chat_id: report.provenance.chat_id.clone(), space_id: report.provenance.space_id.clone() });
        history.sort_by_key(|occurrence| occurrence.at);
        let excess = history.len().saturating_sub(OCCURRENCES_KEPT);
        history.drain(..excess);
        report.occurrences = 1;
        report.occurrence_history.clear();
        report.last_seen = report.provenance.created_at;
        report.status = FrictionStatus::Open;
        let mut fields: BTreeMap<String, Value> = if existing.is_some() {
            BTreeMap::new()
        } else {
            let Value::Object(map) = serde_json::to_value(&report)? else {
                unreachable!()
            };
            map.into_iter().collect()
        };
        fields.insert(counter_key, json!({"count": count, "lastSeen": last_seen, "history": history}));
        self.observe_row(KIND_FRICTION_REPORTS, &report.id);
        let op = RowOp {
            kind: KIND_FRICTION_REPORTS.into(),
            id: report.id.clone(),
            op: if existing.is_some() {
                OpKind::Update
            } else {
                OpKind::Upsert
            },
            set: Some(fields),
            hlc: self.next_hlc(),
            clocks: None,
        };
        check_op_size(&op)?;
        self.write_ops(vec![op])?;
        let report = self
            .overlay_row(KIND_FRICTION_REPORTS, &report.id)
            .as_ref()
            .and_then(decode)
            .ok_or_else(|| DocError::Schema("friction report could not be read".into()))?;
        Ok((report, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mast_proto::{FrictionKind, FrictionProvenance, FrictionSource};

    fn report(space: &str, chat: &str, summary: &str, time: i64) -> FrictionReport {
        FrictionReport {
            id: String::new(),
            kind: FrictionKind::Bug,
            summary: summary.into(),
            what_happened: "Failure".into(),
            workaround: None,
            suggestion: None,
            approval: None,
            severity: None,
            links: vec![],
            provenance: FrictionProvenance {
                chat_id: chat.into(),
                harness: "codex".into(),
                model: None,
                space_id: space.into(),
                work_item_id: None,
                branch: None,
                created_at: time,
                source: FrictionSource::Agent,
                file_name: None,
            },
            fingerprint: String::new(),
            occurrences: 0,
            occurrence_history: Vec::new(),
            last_seen: 0,
            status: FrictionStatus::Open,
            promoted_item_id: None,
            promoted_at: None,
        }
    }

    #[test]
    fn friction_round_trip_and_deduplication() {
        let mut doc = RegistryDoc::new("device");
        let first = doc
            .record_friction(report("space", "chat-1", "Test  failure", 1))
            .unwrap();
        assert_eq!(doc.friction_reports("space"), vec![first.clone()]);
        let mut value = serde_json::to_value(&first).unwrap();
        value.as_object_mut().unwrap().remove("promotedItemId");
        value.as_object_mut().unwrap().remove("promotedAt");
        value.as_object_mut().unwrap().remove("occurrenceHistory");
        let legacy: FrictionReport = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.promoted_item_id, None);
        assert_eq!(legacy.promoted_at, None);
        assert!(legacy.occurrence_history.is_empty());
        let mut promoted = first.clone();
        promoted.promoted_item_id = Some("item".into());
        promoted.promoted_at = Some(42);
        assert_eq!(
            serde_json::from_value::<FrictionReport>(serde_json::to_value(&promoted).unwrap())
                .unwrap(),
            promoted
        );
        let second = doc
            .record_friction(report("space", "chat-2", " test failure ", 2))
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.occurrences, 2);
        assert_eq!(second.last_seen, 2);
        assert_eq!(second.occurrence_history.iter().map(|o| (o.at, o.chat_id.as_str(), o.space_id.as_str())).collect::<Vec<_>>(), vec![(1, "chat-1", "space"), (2, "chat-2", "space")]);
        assert_eq!(second.provenance.chat_id, "chat-1");
        let other = doc
            .record_friction(report("other", "chat-2", "test failure", 3))
            .unwrap();
        assert_ne!(second.id, other.id);
        assert_eq!(doc.friction_reports("space"), vec![second.clone()]);
        let restored = RegistryDoc::from_bytes(&doc.to_bytes().unwrap(), "device").unwrap();
        assert_eq!(restored.friction_reports("space"), vec![second]);
    }
    #[test]
    fn friction_history_keeps_the_latest_fifty_occurrences() {
        let mut doc = RegistryDoc::new("device");
        for at in 0..55 {
            doc.record_friction(report("space", "chat", "failure", at)).unwrap();
        }
        let report = doc.friction_reports("space").pop().unwrap();
        assert_eq!(report.occurrences, 55);
        assert_eq!(report.occurrence_history.len(), 50);
        assert_eq!(report.occurrence_history.first().unwrap().at, 5);
        assert_eq!(report.occurrence_history.last().unwrap().at, 54);
    }

    #[test]
    fn friction_concurrent_devices_preserve_occurrences() {
        let mut first = RegistryDoc::new("device-a");
        let mut second = RegistryDoc::new("device-b");
        let a = first
            .record_friction(report("space", "chat-a", "failure", 1))
            .unwrap();
        second
            .record_friction(report("space", "chat-b", "failure", 2))
            .unwrap();
        let other_op = second.pending[0].ops[0].clone();
        let current = first.overlay_row(KIND_FRICTION_REPORTS, &a.id).unwrap();
        let merged = apply_op(Some(&current), &other_op).0.unwrap();
        let merged = decode(&merged).unwrap();
        assert_eq!(merged.occurrences, 2);
        assert_eq!(merged.last_seen, 2);
        assert_eq!(merged.occurrence_history.len(), 2);
    }
    #[test]
    fn friction_fingerprint_includes_kind_and_command() {
        let base = report("space", "chat", "failure", 1);
        let mut changed = base.clone();
        changed.kind = FrictionKind::Permission;
        assert_ne!(friction_fingerprint(&base), friction_fingerprint(&changed));
        changed = base.clone();
        changed.approval = Some(mast_proto::FrictionApproval {
            command: "cargo test".into(),
            why: "permission".into(),
            answer: None,
        });
        assert_ne!(friction_fingerprint(&base), friction_fingerprint(&changed));
    }
}
