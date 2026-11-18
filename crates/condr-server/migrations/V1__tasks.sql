-- ADR 0068: the Device's Tasks.
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    repository TEXT,
    position INTEGER NOT NULL,
    archived INTEGER NOT NULL,
    workspace INTEGER,
    made_workspace INTEGER NOT NULL,
    last_path TEXT,
    last_branch TEXT,
    last_base_branch TEXT,
    last_managed INTEGER NOT NULL,
    last_repository TEXT
);
