pub const DATA_REPAIRS: &[&str] = &[
    r#"
    UPDATE experiment_projects
    SET owner_user_id = (
        SELECT MIN(experiment_campaigns.owner_user_id)
        FROM experiment_campaigns
        WHERE experiment_campaigns.project_id = experiment_projects.id
    )
    WHERE owner_user_id = 'default'
      AND NOT EXISTS (SELECT 1 FROM _migrations WHERE version = 31)
      AND 1 = (
          SELECT COUNT(DISTINCT experiment_campaigns.owner_user_id)
          FROM experiment_campaigns
          WHERE experiment_campaigns.project_id = experiment_projects.id
      )
    "#,
    r#"
    UPDATE experiment_runner_profiles
    SET owner_user_id = (
        SELECT MIN(experiment_campaigns.owner_user_id)
        FROM experiment_campaigns
        WHERE experiment_campaigns.runner_profile_id = experiment_runner_profiles.id
    )
    WHERE owner_user_id = 'default'
      AND NOT EXISTS (SELECT 1 FROM _migrations WHERE version = 31)
      AND 1 = (
          SELECT COUNT(DISTINCT experiment_campaigns.owner_user_id)
          FROM experiment_campaigns
          WHERE experiment_campaigns.runner_profile_id = experiment_runner_profiles.id
      )
    "#,
    r#"
    INSERT OR IGNORE INTO _migrations(version, name)
    VALUES (31, 'experiment project and runner ownership')
    "#,
    r#"
    UPDATE conversations
    SET actor_id = COALESCE(NULLIF(actor_id, ''), user_id),
        conversation_scope_id = COALESCE(NULLIF(conversation_scope_id, ''), id),
        conversation_kind = COALESCE(NULLIF(conversation_kind, ''), 'direct'),
        stable_external_conversation_key = COALESCE(
            NULLIF(stable_external_conversation_key, ''),
            channel || ':' || COALESCE(NULLIF(thread_id, ''), id)
        )
    WHERE actor_id IS NULL
       OR actor_id = ''
       OR conversation_scope_id IS NULL
       OR conversation_scope_id = ''
       OR conversation_kind IS NULL
       OR conversation_kind = ''
       OR stable_external_conversation_key IS NULL
       OR stable_external_conversation_key = ''
    "#,
    r#"
    UPDATE agent_jobs
    SET principal_id = CASE
            WHEN principal_id IS NULL OR principal_id = '' OR principal_id = 'default'
                THEN COALESCE(user_id, 'default')
            ELSE principal_id
        END,
        actor_id = CASE
            WHEN actor_id IS NULL OR actor_id = '' OR actor_id = 'default'
                THEN COALESCE(user_id, NULLIF(principal_id, ''), 'default')
            ELSE actor_id
        END
    WHERE principal_id IS NULL
       OR principal_id = ''
       OR actor_id IS NULL
       OR actor_id = ''
       OR actor_id = 'default'
    "#,
    r#"
    UPDATE agent_jobs
    SET credential_grants = COALESCE(NULLIF(description, ''), '[]')
    WHERE source = 'sandbox'
      AND (credential_grants IS NULL OR credential_grants = '' OR credential_grants = '[]')
      AND description LIKE '[%'
    "#,
    r#"
    UPDATE routines
    SET actor_id = CASE
            WHEN actor_id IS NULL OR actor_id = '' OR actor_id = 'default'
                THEN COALESCE(user_id, 'default')
            ELSE actor_id
        END
    WHERE actor_id IS NULL
       OR actor_id = ''
       OR actor_id = 'default'
    "#,
    r#"
    UPDATE routine_event_inbox
    SET idempotency_key = COALESCE(NULLIF(idempotency_key, ''), id),
        event_type = COALESCE(NULLIF(event_type, ''), 'message'),
        attempt_count = COALESCE(attempt_count, 0)
    WHERE idempotency_key IS NULL
       OR idempotency_key = ''
       OR event_type IS NULL
       OR event_type = ''
       OR attempt_count IS NULL
    "#,
    r#"
    UPDATE memory_chunks
    SET embedding_blob = COALESCE(embedding_blob, embedding),
        embedding_dim = COALESCE(embedding_dim, 1536)
    WHERE embedding IS NOT NULL
      AND (embedding_blob IS NULL OR embedding_dim IS NULL)
    "#,
    r#"
    WITH ordered_documents AS (
        SELECT *
        FROM memory_documents
        WHERE agent_id IS NULL
        ORDER BY created_at ASC, id ASC
    ), duplicate_groups AS (
        SELECT user_id,
               path,
               MIN(id) AS keep_id,
               group_concat(NULLIF(content, ''), char(10) || char(10)) AS merged_content,
               MAX(updated_at) AS newest_update
        FROM ordered_documents
        GROUP BY user_id, path
        HAVING COUNT(*) > 1
    )
    UPDATE memory_documents
    SET content = COALESCE(
            (SELECT merged_content FROM duplicate_groups WHERE keep_id = memory_documents.id),
            ''
        ),
        updated_at = COALESCE(
            (SELECT newest_update FROM duplicate_groups WHERE keep_id = memory_documents.id),
            updated_at
        ),
        metadata = json_set(
            CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
            '$.index_dirty', json('true')
        )
    WHERE id IN (SELECT keep_id FROM duplicate_groups)
    "#,
    r#"
    DELETE FROM memory_chunks
    WHERE document_id IN (
        SELECT document.id
        FROM memory_documents AS document
        JOIN (
            SELECT user_id, path
            FROM memory_documents
            WHERE agent_id IS NULL
            GROUP BY user_id, path
            HAVING COUNT(*) > 1
        ) AS duplicate
          ON duplicate.user_id = document.user_id
         AND duplicate.path = document.path
        WHERE document.agent_id IS NULL
    )
    "#,
    r#"
    DELETE FROM memory_documents
    WHERE agent_id IS NULL
      AND id NOT IN (
          SELECT MIN(id)
          FROM memory_documents
          WHERE agent_id IS NULL
          GROUP BY user_id, path
      )
    "#,
    r#"
    CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_shared_path_unique
    ON memory_documents(user_id, path)
    WHERE agent_id IS NULL
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_conversations_ingress_direct
    ON conversations(user_id, actor_id, channel, thread_id, last_activity DESC)
    WHERE conversation_kind = 'direct'
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_conversations_ingress_group
    ON conversations(user_id, conversation_scope_id, last_activity DESC)
    WHERE conversation_kind = 'group'
    "#,
    r#"
    UPDATE memory_documents
    SET metadata = json_set(
        CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
        '$.index_dirty', json('true')
    )
    WHERE json_extract(
        CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
        '$.index_dirty'
    ) IS NULL
    "#,
    r#"
    INSERT INTO conversation_messages_fts(conversation_messages_fts)
    VALUES ('rebuild')
    "#,
];
