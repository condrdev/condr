# Core Owns Layout State

`murmur-core` owns stable Session, Workspace, Tab, and Pane identities plus the Tab split layout; `murmur-gui` projects the active layout into gpui-component Dock. Dock runtime IDs, tab stacks, and serialized state are not domain or persistence truth because their lifecycle does not expose every close/focus transition and tying saved state to GPUI would obstruct the planned server/client split; GUI layout mutations therefore route through core and Dock remains a replaceable view.
