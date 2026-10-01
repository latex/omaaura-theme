# Backends de Hardware do OmaAura

O OmaAura é **agnóstico de hardware**. O núcleo (calibração de cor, tema, CLI,
widget, daemon) só orquestra **backends**. Cada hardware suportado — placa-mãe,
GPU, microfone, teclado, fita de LED… — é um backend plugável.

Existem **dois tipos** de backend:

| Tipo | Quem escreve | Linguagem | Como entrega |
| :--- | :--- | :--- | :--- |
| **Interno** | mantenedores do crate | Rust | módulo em `src/backend/builtin/` |
| **Externo** | **qualquer dev** | **qualquer uma** | executável + `backend.toml` |

---

## 1. Contrato (trait `Backend`)

Backends internos implementam:

```rust
pub trait Backend: Send + Sync {
    fn id(&self) -> String;                     // "openrgb", "ene_i2c", "fifine"
    fn name(&self) -> String;
    fn kinds(&self) -> Vec<DeviceKind>;         // Gpu | Motherboard | Ram | Mic | Deck | Keyboard | Mouse | Headset | Light | Generic
    fn version(&self) -> Option<String> { None }

    /// Apenas os dispositivos CONTROLÁVEIS presentes agora.
    fn detect(&self, enabled: bool) -> Vec<DetectedDevice>;

    /// Aplica a cor em todos os dispositivos deste backend.
    fn apply(&self, color: &Color, params: &toml::Value) -> Result<ApplyOutcome>;

    /// Apaga todos os dispositivos deste backend.
    fn off(&self, params: &toml::Value) -> Result<ApplyOutcome>;
}
```

`Color` é um `rrggbb` minúsculo já **calibrado** (sem `#`). `params` é a tabela
do backend em `config.toml` (ex.: `[openrgb]`).

Para adicionar um backend interno:

1. Crie `src/backend/builtin/meu_hardware.rs` implementando `Backend`.
2. Registre em `src/backend/builtin/mod.rs` → `builtin_backends()`.
3. Adicione o `id` em `backends.order` do `config.toml`.

---

## 2. Backends externos (qualquer linguagem)

Um backend externo é **um executável** que fala um protocolo JSON simples.
Não precisa forkar nem recompilar o OmaAura.

### 2.1 Manifest (`backend.toml`)

```toml
id          = "fifine"
name        = "FIFINE Microphone"
version     = "1.0.0"
kinds       = ["mic"]
exec        = "omaaura-backend-fifine"   # PATH ou caminho relativo ao manifest
```

### 2.2 Descoberta

O OmaAura procura manifests em `~/.config/omaaura/backends/`:

```
~/.config/omaaura/backends/
├── fifine/
│   ├── backend.toml
│   └── omaaura-backend-fifine      # executável (relativo ao manifest)
└── meu-teclado.toml                # ou manifest "flat" + exec no PATH
```

### 2.3 Protocolo

O `exec` é chamado com um subcomando e deve responder **JSON no stdout**:

```bash
# 1) Descobrir dispositivos controláveis
$ omaaura-backend-fifine detect
{"devices":[{"id":"3142:a010","name":"FIFINE","kinds":["mic"]}]}

# 2) Aplicar uma cor (rrggbb, sem '#')
$ omaaura-backend-fifine apply 0080ff
{"applied":true,"devices":1}

# 3) Apagar
$ omaaura-backend-fifine off
{"applied":true}
```

- **Exit code 0** = sucesso; qualquer outro é reportado como falha (o OmaAura avisa e segue).
- Timeout por chamada: **30s**.
- A tabela de config do backend é entregue no env **`OMAAURA_PARAMS`** (JSON).
- Variáveis disponíveis: `OMAAURA_BACKEND_ID`, `OMAAURA_PARAMS`, `OMAAURA_COLOR`.
- `detect` deve ser **rápido e sem efeitos colaterais**.

### 2.4 Configuração

Habilite e ordene o backend em `~/.config/omaaura/config.toml`:

```toml
[backends]
order = ["openrgb", "ene_i2c", "fifine"]   # ordem de aplicação

[fifine]
brightness = 100          # vira o OMAAURA_PARAMS para o seu executável
```

> A ordem importa: o OmaAura aplica **na sequência**. Mantenha a GPU por último
> para que o I2C direto vença qualquer disputa.

---

## 3. Comandos úteis

```bash
omaaura backends          # lista backends registrados (id, versão, ativos, tipos)
omaaura devices           # lista dispositivos controláveis (todos os backends)
omaaura devices --json    # idem, em JSON
omaaura probe             # identifica USB e classifica o RGB (controlável/só-botão/não-RGB)
sudo omaaura probe --deep # + strings ocultas + protocolo vendor (ex.: C-Media)
omaaura sync              # aplica a cor do tema em todos os backends ativos
omaaura test-hardware     # diagnóstico (I2C + OpenRGB + inventário)
```

> **Antes de escrever um backend**, rode `omaaura probe`: ele diz se o hardware
> expõe algum canal de LED (HID LED/LampArray, interface vendor, etc.). Se o
> veredito for *"SÓ BOTÃO"*, não há backend possível.

---

## 4. Exemplo completo (shell)

Veja `examples/backends/example/` neste repositório: um backend de brinquedo que
"controla" um LED virtual, útil como template.

```bash
cp -r examples/backends/example ~/.config/omaaura/backends/example
chmod +x ~/.config/omaaura/backends/example/omaaura-backend-example
# adicione "example" em backends.order e rode:
omaaura devices
```

---

## 5. Diretrizes de qualidade

- **Zero efeitos colaterais no `detect`.**
- **Nunca** aplicar cor em dispositivos que não são seus (respeite a fronteira de código de outros devs).
- Trate ausência de hardware com `{"applied":false,"message":"..."}` — não quebre a cadeia.
- Se o seu device precisar de acesso especial (`/dev/hidraw*`, I²C, USB), documente a regra udev/grupo necessária e **nunca** eleve privilégio silenciosamente.
- Escreva testes para o parsing do protocolo do seu device.
