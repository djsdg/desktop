-- Ora SQLite Schema
-- Constraints: No foreign keys, numeric category fields.

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    root_path TEXT NOT NULL
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status INTEGER NOT NULL DEFAULT 0, -- 0: todo, 1: doing, 2: done
    worktree_id TEXT
);

CREATE TABLE worktrees (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    branch_name TEXT,
    is_active INTEGER DEFAULT 0 -- 0: inactive, 1: active
);

CREATE TABLE virtual_folders (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    mount_point TEXT NOT NULL
);

CREATE TABLE virtual_entries (
    id TEXT PRIMARY KEY,
    virtual_folder_id TEXT NOT NULL,
    parent_entry_id TEXT, -- Nullable for root entries
    name TEXT NOT NULL,
    kind INTEGER NOT NULL DEFAULT 0, -- 0: file, 1: directory
    content_ref TEXT -- Artifact UUID when kind is file
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    agent_session_id TEXT,
    status INTEGER NOT NULL DEFAULT 0 -- 0: running, 1: stopped
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    content TEXT
);
