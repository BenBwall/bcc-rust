use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::Duration as StdDuration,
};

use chrono::{
    Duration,
    SecondsFormat,
    Utc,
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait,
    Condition,
    ConnectOptions,
    ConnectionTrait,
    Database,
    DatabaseConnection,
    DatabaseTransaction,
    DbBackend,
    EntityTrait,
    ModelTrait,
    PaginatorTrait,
    QueryFilter,
    QueryOrder,
    QuerySelect,
    Schema,
    Select,
    SqliteTransactionMode,
    TransactionOptions,
    TransactionTrait,
    sea_query::{
        Expr,
        ExprTrait,
        Index,
        OnConflict,
        Query,
    },
    sqlx::sqlite::SqliteJournalMode,
};
use serde_json::{
    Value,
    json,
};

use super::{
    AFK_CONTINUATIONS_PER_HOUR,
    BROADCAST,
    EVERY_AGENT,
    HOOK_MESSAGE_LIMIT,
    Result,
    SessionHooks,
    now,
    validate_agent,
};
use crate::entities::{
    agent_state,
    claims,
    messages,
    receipts,
};

fn db_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn message_json(model: messages::Model) -> Value {
    serde_json::to_value(model).expect("serializable message")
}
fn claim_json(model: claims::Model) -> Value {
    serde_json::to_value(model).expect("serializable claim")
}

#[derive(Copy, Clone)]
pub enum DeliveryFilter {
    All,
    Undelivered,
    Unacked,
    DeliveredUnacked,
}

fn addressed_query(agent: &str, filter: DeliveryFilter) -> Select<messages::Entity> {
    let mut query = messages::Entity::find().filter(
        Condition::any()
            .add(messages::Column::Recipient.eq(agent))
            .add(
                Condition::all()
                    .add(messages::Column::Recipient.eq(BROADCAST))
                    .add(messages::Column::Sender.ne(agent)),
            ),
    );
    if !matches!(filter, DeliveryFilter::All) {
        let mut receipts_query = Query::select();
        receipts_query
            .column(receipts::Column::MessageId)
            .from(receipts::Entity)
            .and_where(receipts::Column::Agent.eq(agent));
        let condition = match filter {
            | DeliveryFilter::Undelivered => {
                receipts_query.and_where(receipts::Column::DeliveredAt.is_not_null());
                Expr::col(messages::Column::Id).not_in_subquery(receipts_query.to_owned())
            },
            | DeliveryFilter::Unacked => {
                receipts_query.and_where(receipts::Column::AckedAt.is_not_null());
                Expr::col(messages::Column::Id).not_in_subquery(receipts_query.to_owned())
            },
            | DeliveryFilter::DeliveredUnacked => {
                receipts_query
                    .and_where(receipts::Column::DeliveredAt.is_not_null())
                    .and_where(receipts::Column::AckedAt.is_null());
                Expr::col(messages::Column::Id).in_subquery(receipts_query.to_owned())
            },
            | DeliveryFilter::All => unreachable!(),
        };
        query = query.filter(condition);
    }
    query
}

pub struct Bus {
    pub path:  PathBuf,
    pub hooks: SessionHooks,
    db:        DatabaseConnection,
}
impl Bus {
    async fn begin(&self) -> Result<DatabaseTransaction> {
        self.db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .map_err(db_error)
    }

    pub async fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(db_error)?;
        }
        let db_path = path.clone();
        let mut options = ConnectOptions::new("sqlite://agentbus.db");
        options
            .max_connections(1)
            .min_connections(1)
            .connect_timeout(StdDuration::from_secs(10))
            .acquire_timeout(StdDuration::from_secs(10))
            .sqlx_logging(false)
            .map_sqlx_sqlite_opts(move |sqlite| {
                sqlite
                    .filename(&db_path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(StdDuration::from_secs(10))
            });
        let db = Database::connect(options).await.map_err(db_error)?;
        let schema = Schema::new(DbBackend::Sqlite);
        for table in [
            schema.create_table_from_entity(messages::Entity),
            schema.create_table_from_entity(receipts::Entity),
            schema.create_table_from_entity(claims::Entity),
            schema.create_table_from_entity(agent_state::Entity),
        ] {
            let mut table = table;
            table.if_not_exists();
            db.execute(&table).await.map_err(db_error)?;
        }
        let index = Index::create()
            .name("messages_recipient")
            .table(messages::Entity)
            .col(messages::Column::Recipient)
            .col(messages::Column::Id)
            .if_not_exists()
            .to_owned();
        db.execute(&index).await.map_err(db_error)?;
        Ok(Self {
            hooks: SessionHooks::new(&path),
            path,
            db,
        })
    }

    pub async fn send(
        &self,
        sender: &str,
        recipient: &str,
        body: &str,
        subject: &str,
        reply_to: Option<i64>,
    ) -> Result<Value> {
        let sender = validate_agent(sender, false)?;
        let recipient = validate_agent(recipient, true)?;
        if sender == recipient {
            return Err("cannot send a message to yourself".into());
        }
        if body.trim().is_empty() {
            return Err("message body is empty".into());
        }
        let transaction = self.begin().await?;
        let thread_id = if let Some(parent) = reply_to {
            let parent_message = messages::Entity::find_by_id(parent)
                .one(&transaction)
                .await
                .map_err(db_error)?
                .ok_or_else(|| format!("no message #{parent} to reply to"))?;
            Some(parent_message.thread_id.unwrap_or(parent_message.id))
        } else {
            None
        };
        let model = messages::ActiveModel {
            created_at: Set(now()),
            sender: Set(sender),
            recipient: Set(recipient),
            thread_id: Set(thread_id),
            subject: Set(subject.trim().to_string()),
            body: Set(body.to_string()),
            ..Default::default()
        }
        .insert(&transaction)
        .await
        .map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        Ok(message_json(model))
    }

    pub async fn message(&self, id: i64) -> Result<Value> {
        messages::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(message_json)
            .ok_or_else(|| format!("no message #{id}"))
    }

    async fn addressed_on<C: ConnectionTrait>(
        &self,
        db: &C,
        agent: &str,
        filter: DeliveryFilter,
        limit: Option<usize>,
    ) -> Result<Vec<Value>> {
        let mut query = addressed_query(agent, filter).order_by_asc(messages::Column::Id);
        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }
        let models = query.all(db).await.map_err(db_error)?;
        let ids: Vec<i64> = models.iter().map(|m| m.id).collect();
        let receipt_map: HashMap<i64, receipts::Model> = if ids.is_empty() {
            HashMap::new()
        } else {
            receipts::Entity::find()
                .filter(receipts::Column::Agent.eq(agent))
                .filter(receipts::Column::MessageId.is_in(ids))
                .all(db)
                .await
                .map_err(db_error)?
                .into_iter()
                .map(|r| (r.message_id, r))
                .collect()
        };
        let mut output = Vec::new();
        for model in models {
            let receipt = receipt_map.get(&model.id);
            let delivered = receipt.and_then(|r| r.delivered_at.as_ref());
            let acked = receipt.and_then(|r| r.acked_at.as_ref());
            let mut value = message_json(model);
            value["delivered_at"] = json!(delivered);
            value["acked_at"] = json!(acked);
            output.push(value);
        }
        Ok(output)
    }

    pub async fn addressed_to(
        &self,
        agent: &str,
        filter: DeliveryFilter,
        limit: Option<usize>,
    ) -> Result<Vec<Value>> {
        self.addressed_on(&self.db, agent, filter, limit).await
    }

    async fn mark<C: ConnectionTrait>(
        &self,
        db: &C,
        agent: &str,
        ids: &[i64],
        ack: bool,
    ) -> Result<()> {
        for id in ids {
            let existing = receipts::Entity::find_by_id((*id, agent.to_string()))
                .one(db)
                .await
                .map_err(db_error)?;
            let stamp = now();
            if let Some(existing) = existing {
                let mut active: receipts::ActiveModel = existing.clone().into();
                if existing.delivered_at.is_none() {
                    active.delivered_at = Set(Some(stamp.clone()));
                }
                if ack && existing.acked_at.is_none() {
                    active.acked_at = Set(Some(stamp));
                }
                active.update(db).await.map_err(db_error)?;
            } else {
                receipts::ActiveModel {
                    message_id:   Set(*id),
                    agent:        Set(agent.to_string()),
                    delivered_at: Set(Some(stamp.clone())),
                    acked_at:     Set(ack.then_some(stamp)),
                }
                .insert(db)
                .await
                .map_err(db_error)?;
            }
        }
        Ok(())
    }

    pub async fn take_undelivered(&self, agent: &str) -> Result<(Vec<Value>, usize)> {
        let agent = validate_agent(agent, false)?;
        let transaction = self.begin().await?;
        let count = addressed_query(&agent, DeliveryFilter::Undelivered)
            .count(&transaction)
            .await
            .map_err(db_error)? as usize;
        let remaining = count.saturating_sub(HOOK_MESSAGE_LIMIT);
        let taken = self
            .addressed_on(
                &transaction,
                &agent,
                DeliveryFilter::Undelivered,
                Some(HOOK_MESSAGE_LIMIT),
            )
            .await?;
        let ids: Vec<i64> = taken.iter().filter_map(|m| m["id"].as_i64()).collect();
        self.mark(&transaction, &agent, &ids, false).await?;
        transaction.commit().await.map_err(db_error)?;
        Ok((taken, remaining))
    }

    pub async fn inbox(
        &self,
        agent: &str,
        include_acked: bool,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let agent = validate_agent(agent, false)?;
        let transaction = self.begin().await?;
        let mut messages = self
            .addressed_on(
                &transaction,
                &agent,
                if include_acked {
                    DeliveryFilter::All
                } else {
                    DeliveryFilter::Unacked
                },
                if include_acked { None } else { Some(limit) },
            )
            .await?;
        if include_acked && messages.len() > limit {
            messages.drain(..messages.len() - limit);
        }
        let ids: Vec<i64> = messages.iter().filter_map(|m| m["id"].as_i64()).collect();
        self.mark(&transaction, &agent, &ids, false).await?;
        transaction.commit().await.map_err(db_error)?;
        Ok(messages)
    }

    pub async fn ack(&self, agent: &str, ids: &[i64]) -> Result<Vec<i64>> {
        let agent = validate_agent(agent, false)?;
        let transaction = self.begin().await?;
        let visible: Vec<i64> = addressed_query(&agent, DeliveryFilter::All)
            .filter(messages::Column::Id.is_in(ids.to_vec()))
            .all(&transaction)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|m| m.id)
            .collect();
        let mut unknown: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| !visible.contains(id))
            .collect();
        unknown.sort_unstable();
        unknown.dedup();
        if !unknown.is_empty() {
            return Err(format!("messages not addressed to {agent}: {unknown:?}"));
        }
        self.mark(&transaction, &agent, ids, true).await?;
        transaction.commit().await.map_err(db_error)?;
        let mut result = ids.to_vec();
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }

    pub async fn thread(&self, id: i64) -> Result<Vec<Value>> {
        let root = self.message(id).await?;
        let root_id = root["thread_id"].as_i64().unwrap_or(id);
        Ok(messages::Entity::find()
            .filter(
                Condition::any()
                    .add(messages::Column::Id.eq(root_id))
                    .add(messages::Column::ThreadId.eq(root_id)),
            )
            .order_by_asc(messages::Column::Id)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(message_json)
            .collect())
    }

    pub async fn log(&self, limit: i64) -> Result<Vec<Value>> {
        let mut rows: Vec<Value> = messages::Entity::find()
            .order_by_desc(messages::Column::Id)
            .limit(std::cmp::max(limit, 0) as u64)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(message_json)
            .collect();
        rows.reverse();
        Ok(rows)
    }

    pub async fn claim(
        &self,
        agent: &str,
        resource: &str,
        note: &str,
        ttl_minutes: i64,
    ) -> Result<Value> {
        let agent = validate_agent(agent, false)?;
        let resource = resource.trim();
        if resource.is_empty() {
            return Err("resource is empty".into());
        }
        if !(1..=1440).contains(&ttl_minutes) {
            return Err("ttl_minutes must be between 1 and 1440".into());
        }
        let transaction = self.begin().await?;
        let held = claims::Entity::find_by_id(resource.to_string())
            .one(&transaction)
            .await
            .map_err(db_error)?;
        let moment = now();
        if let Some(held) = &held
            && held.agent != agent
            && held.expires_at > moment
        {
            return Err(format!(
                "{resource:?} is claimed by {} until {}{}",
                held.agent,
                held.expires_at,
                if held.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", held.note)
                }
            ));
        }
        let until = (Utc::now() + Duration::minutes(ttl_minutes))
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        let model = if let Some(existing) = held {
            let mut active: claims::ActiveModel = existing.into();
            active.agent = Set(agent);
            active.note = Set(note.trim().to_string());
            active.claimed_at = Set(moment);
            active.expires_at = Set(until);
            active.update(&transaction).await.map_err(db_error)?
        } else {
            claims::ActiveModel {
                resource:   Set(resource.to_string()),
                agent:      Set(agent),
                note:       Set(note.trim().to_string()),
                claimed_at: Set(moment),
                expires_at: Set(until),
            }
            .insert(&transaction)
            .await
            .map_err(db_error)?
        };
        transaction.commit().await.map_err(db_error)?;
        Ok(claim_json(model))
    }

    pub async fn release(&self, agent: &str, resource: &str) -> Result<bool> {
        let agent = validate_agent(agent, false)?;
        let transaction = self.begin().await?;
        let held = claims::Entity::find_by_id(resource.trim().to_string())
            .one(&transaction)
            .await
            .map_err(db_error)?;
        let Some(held) = held else { return Ok(false) };
        if held.agent != agent {
            return Err(format!(
                "{resource:?} is claimed by {}, not {agent}",
                held.agent
            ));
        }
        held.delete(&transaction).await.map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        Ok(true)
    }

    pub async fn claims(&self) -> Result<Vec<Value>> {
        Ok(claims::Entity::find()
            .filter(claims::Column::ExpiresAt.gt(now()))
            .order_by_asc(claims::Column::Resource)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(claim_json)
            .collect())
    }

    pub async fn get_state(&self, agent: &str, key: &str) -> Result<String> {
        Ok(
            agent_state::Entity::find_by_id((agent.to_string(), key.to_string()))
                .one(&self.db)
                .await
                .map_err(db_error)?
                .map(|m| m.value)
                .unwrap_or_default(),
        )
    }

    pub async fn set_state(&self, agent: &str, key: &str, value: &str) -> Result<()> {
        agent_state::Entity::insert(agent_state::ActiveModel {
            agent: Set(agent.to_string()),
            key:   Set(key.to_string()),
            value: Set(value.to_string()),
        })
        .on_conflict(
            OnConflict::columns([agent_state::Column::Agent, agent_state::Column::Key])
                .update_column(agent_state::Column::Value)
                .to_owned(),
        )
        .exec(&self.db)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    pub async fn redeliver(&self, agent: &str, ids: &[i64]) -> Result<Vec<i64>> {
        let agent = validate_agent(agent, false)?;
        let transaction = self.begin().await?;
        let mut reset = Vec::new();
        for id in ids {
            let existing = receipts::Entity::find_by_id((*id, agent.clone()))
                .one(&transaction)
                .await
                .map_err(db_error)?;
            if let Some(existing) = existing
                && existing.acked_at.is_none()
            {
                let mut active: receipts::ActiveModel = existing.into();
                active.delivered_at = Set(None);
                active.update(&transaction).await.map_err(db_error)?;
                reset.push(*id);
            }
        }
        transaction.commit().await.map_err(db_error)?;
        Ok(reset)
    }

    pub async fn set_afk(&self, until: Option<String>, agent: &str) -> Result<()> {
        if agent == EVERY_AGENT {
            if let Some(until) = until {
                self.set_state(agent, "afk_until", &until).await?;
            } else {
                agent_state::Entity::delete_many()
                    .filter(agent_state::Column::Key.eq("afk_until"))
                    .exec(&self.db)
                    .await
                    .map_err(db_error)?;
            }
        } else {
            let agent = validate_agent(agent, false)?;
            self.set_state(&agent, "afk_until", until.as_deref().unwrap_or("off"))
                .await?;
        }
        Ok(())
    }

    pub async fn afk_until(&self, agent: &str) -> Result<Option<String>> {
        for scope in [agent, EVERY_AGENT] {
            let value = self.get_state(scope, "afk_until").await?;
            if value == "off" {
                return Ok(None);
            }
            if !value.is_empty() {
                return Ok((value > now()).then_some(value));
            }
        }
        Ok(None)
    }

    pub async fn afk_settings(&self) -> Result<Vec<Value>> {
        Ok(agent_state::Entity::find()
            .filter(agent_state::Column::Key.eq("afk_until"))
            .order_by_asc(agent_state::Column::Agent)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|m| json!({"agent":m.agent,"value":m.value}))
            .collect())
    }

    pub async fn afk_allowance(&self, agent: &str) -> Result<usize> {
        Ok(
            AFK_CONTINUATIONS_PER_HOUR
                .saturating_sub(self.recent_continuations(agent).await?.len()),
        )
    }

    pub async fn record_afk_continuation(&self, agent: &str) -> Result<()> {
        let mut recent = self.recent_continuations(agent).await?;
        recent.push(now());
        self.set_state(
            agent,
            "afk_continuations",
            &serde_json::to_string(&recent).map_err(db_error)?,
        )
        .await
    }

    async fn recent_continuations(&self, agent: &str) -> Result<Vec<String>> {
        let horizon = (Utc::now() - Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
        let state = self.get_state(agent, "afk_continuations").await?;
        let values: Vec<String> =
            serde_json::from_str(if state.is_empty() { "[]" } else { &state }).map_err(db_error)?;
        Ok(values.into_iter().filter(|v| v > &horizon).collect())
    }
}
