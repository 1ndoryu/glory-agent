# glory-agent

Chat en tiempo real + IA (Rust, Axum + SQLx + providers OpenAI-compat).
Primer provider: `opencode-go` (`muse-spark-1.3-contribuidor`, ventana 30k).

## Uso mínimo (consumidor)

```rust
use glory_agent::{prompts::PromptConfig, providers::ProviderConfig, transport::{routes, AgentState}};

let state = AgentState::new(
    ProviderConfig { base_url: std::env::var("OPENCODE_GO_BASE_URL")?,
        api_key: std::env::var("OPENCODE_GO_API_KEY")?,
        model: "muse-spark-1.3-contribuidor".into() },
    PromptConfig::inmobiliaria_ejemplo(),
);
let app = routes().with_state(state);
```

## Env

Ver `.env.example`. Claves solo en server, nunca al front.

## Comandos

```powershell
$env:CARGO_TARGET_DIR = "C:\tmp\glory-target\glory-agent"
cargo fmt --check; cargo check; cargo clippy --all-targets -- -D warnings; cargo test
sentinel doctor --workspace .
```
