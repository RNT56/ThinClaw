-- Canonical Direct Workbench message attachments.
--
-- Legacy messages.images and messages.attached_docs remain readable for old
-- profiles. New writes populate messages.assets with AssetRef[] and may mirror
-- legacy columns only for frontend compatibility during migration.

ALTER TABLE messages ADD COLUMN assets TEXT;

UPDATE messages
SET assets = (
    SELECT json_group_array(json(asset_ref))
    FROM (
        SELECT json_object(
            'namespace', 'direct_workbench',
            'id', value
        ) AS asset_ref
        FROM json_each(CASE WHEN json_valid(messages.images) THEN messages.images ELSE '[]' END)

        UNION ALL

        SELECT json_object(
            'namespace', 'direct_workbench',
            'id', COALESCE(json_extract(value, '$.assetRef.id'), json_extract(value, '$.asset_ref.id'), json_extract(value, '$.id'))
        ) AS asset_ref
        FROM json_each(CASE WHEN json_valid(messages.attached_docs) THEN messages.attached_docs ELSE '[]' END)
        WHERE COALESCE(json_extract(value, '$.assetRef.id'), json_extract(value, '$.asset_ref.id'), json_extract(value, '$.id')) IS NOT NULL
    )
)
WHERE assets IS NULL
  AND (
    (images IS NOT NULL AND images != '' AND json_valid(images))
    OR (attached_docs IS NOT NULL AND attached_docs != '' AND json_valid(attached_docs))
  );
