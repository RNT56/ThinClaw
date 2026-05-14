-- Add sort_order to conversations and projects
ALTER TABLE conversations ADD COLUMN sort_order INTEGER DEFAULT 0;
ALTER TABLE projects ADD COLUMN sort_order INTEGER DEFAULT 0;
