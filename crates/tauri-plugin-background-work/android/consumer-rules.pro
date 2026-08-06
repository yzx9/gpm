# Consumer ProGuard rules for tauri-plugin-background-work.
#
# This plugin is worker-agnostic: it carries no Worker class of its own (the
# caller supplies the worker FQN at schedule time). The app that owns the
# Worker keeps its own `-keep` rule in its proguard-rules.pro.
