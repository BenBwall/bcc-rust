pub mod messages {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize)]
    #[sea_orm(table_name = "messages")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id:         i64,
        pub created_at: String,
        pub sender:     String,
        pub recipient:  String,
        pub thread_id:  Option<i64>,
        pub subject:    String,
        pub body:       String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "super::receipts::Entity")]
        Receipts,
    }
    impl Related<super::receipts::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Receipts.def()
        }
    }
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod receipts {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize)]
    #[sea_orm(table_name = "receipts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub message_id:   i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub agent:        String,
        pub delivered_at: Option<String>,
        pub acked_at:     Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::messages::Entity",
            from = "Column::MessageId",
            to = "super::messages::Column::Id"
        )]
        Message,
    }
    impl Related<super::messages::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Message.def()
        }
    }
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod claims {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize)]
    #[sea_orm(table_name = "claims")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub resource:   String,
        pub agent:      String,
        pub note:       String,
        pub claimed_at: String,
        pub expires_at: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod agent_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize)]
    #[sea_orm(table_name = "agent_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub agent: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub key:   String,
        pub value: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
