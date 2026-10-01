# OmaAura Theme

Plugin para o [Omarchy Linux](https://omarchy.org/) que sincroniza automaticamente as cores dos LEDs de hardware (ASUS Aura, placas-mãe ROG/TUF e placas de vídeo GeForce RTX) com o tema visual ativo da área de trabalho.

`id`: `io.github.latex.omaaura-theme` · categoria: **Hardware** · versão: **2.0.0 (Rust)**

> **v2 (2026):** reescrito de Python para um **binário Rust** único. Zero dependência de Python.
> Corrige o defeito apontado na revisão do marketplace: o backend OpenRGB **nunca** mais roda
> `openrgb -c` sem filtro `-d` — se os alvos não forem resolvidos, a escrita é ignorada em vez de
> atingir todos os dispositivos (inclusive a GPU controlada por I2C).

---

## 💡 Recursos

- **Sincronização com o Tema:** Sempre que você troca o tema (`omarchy theme set`), a cor de destaque (`accent`) é imediatamente aplicada aos LEDs do hardware em segundo plano.
- **Detecção Inteligente do Wallpaper:** Extrai automaticamente a cor predominante e os destaques vibrantes do papel de parede ativo (`~/.local/state/omarchy/current/background`). Trocar o wallpaper atualiza a paleta na hora.
- **Menu Popup Interativo na Barra (`PopupCard`):**
  - **Clique Esquerdo no Ícone:** Abre o seletor com as cores do tema Omarchy e as cores dominantes do wallpaper atual.
  - **Clique em Qualquer Cor:** Aplica imediatamente a cor escolhida diretamente nos LEDs de hardware (GPU TUF RTX e Placa-mãe ROG B550-F + Fans ARGB).
  - **Clique Direito no Ícone:** Liga / Desliga a iluminação sem abrir menus.
- **CLI Integrada:** Utilitário `bin/omaaura` — **binário único em Rust** (v2) para controle via terminal e scripts. `bin/omaaura-theme` é apenas um wrapper de compatibilidade.
- **Calibração de Cor para LEDs:** aplica piso de saturação/valor para que um tom pastel do tema (ex.: `#ed8796`) apareça como uma cor viva no LED em vez de "branco com um pouco de cor". O swatch do popup mostra exatamente a cor calibrada que será enviada ao hardware (ícone = LED).
- **Sem conflito de backends:** a GPU é controlada exclusivamente por I2C direto (ENE Aura) e o OpenRGB atua **apenas** na placa-mãe, RAM e headers ARGB — nunca na GPU.
- **Compatível com OpenRGB:** Suporta motherboards ASUS Aura, GPUs, memórias RAM e headers ARGB.



---

## 🛠️ Requisitos

- **Omarchy Linux** (com `omarchy-shell` / Quickshell).
- **OpenRGB** — controle da placa-mãe Aura, RAM e headers ARGB: `omarchy pkg add openrgb`.
- **Rust / Cargo** (≥ 1.85) — o CLI é um binário Rust compilado por `install.sh`: `omarchy pkg add rust`.
- **ImageMagick** (`magick`) — extração das cores predominantes do wallpaper: `omarchy pkg add imagemagick`.
- **Acesso I2C** (`/dev/i2c-*`) para a GPU via ENE Aura. Adicione seu usuário ao grupo `i2c` (exige logout/login):
  ```bash
  sudo usermod -aG i2c "$USER"
  ```
- **systemd --user** — o serviço `omaaura.service` mantém os LEDs sincronizados em segundo plano.

---

## 🚀 Instalação

### Via Omarchy Plugin Manager (recomendado)
```bash
omarchy plugin add https://github.com/latex/omaaura-theme.git --enable --yes
```

Depois compile o binário e rode o assistente de hardware (detecta GPU/OpenRGB,
grava `~/.config/omaaura/config.toml` e instala o serviço systemd + hooks):

```bash
PLUGIN=~/.config/omarchy/plugins/io.github.latex.omaaura-theme
( cd "$PLUGIN" && cargo build --release )
cp "$PLUGIN"/target/release/omaaura "$PLUGIN"/bin/omaaura-bin

"$PLUGIN"/bin/omaaura setup                          # config + serviço + hooks
ln -sf "$PLUGIN"/bin/omaaura ~/.local/bin/omaaura    # CLI no PATH (opcional)
```

### Via repositório clonado (desenvolvimento)
```bash
git clone https://github.com/latex/omaaura-theme.git
cd omaaura-theme && ./install.sh
```

> **Nota de segurança:** plugins Omarchy rodam sem sandbox dentro do processo
> `omarchy-shell`. Este plugin cria hooks em `~/.config/omarchy/hooks/`, links em
> `~/.local/bin` e um serviço `systemd --user`; nada além do `omarchy pkg add`
> das dependências usa privilégio elevado — a entrada no grupo `i2c` é manual.

---

## 🗑️ Remoção

```bash
omarchy plugin remove io.github.latex.omaaura-theme --yes   # desativa e apaga o plugin
systemctl --user disable --now omaaura.service 2>/dev/null || true
rm -f ~/.local/bin/omaaura ~/.local/bin/omaaura-theme
rm -f ~/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh \
      ~/.config/omarchy/hooks/post-boot.d/omaaura-theme.sh
rm -rf ~/.config/omaaura ~/.local/state/omaaura-theme
```

---

## 💻 Uso via Linha de Comando (CLI)

O utilitário `bin/omaaura` (ou o wrapper `bin/omaaura-theme`) pode ser executado diretamente:

```bash
# Sincroniza com a cor do tema Omarchy atual
omaaura sync

# Alterna entre ligado e desligado
omaaura toggle

# Desliga os LEDs
omaaura off

# Aplica uma cor hexadecimal específica (passa pela calibração de LED)
omaaura set ed8796

# Mostra o status atual (cor calibrada)
omaaura status
```

### 🎨 Calibração de Cor (`~/.config/omaaura/config.toml`)

Monitores e LEDs emitem luz de forma diferente: tons pastel que parecem vivos na
tela viram "branco com um resto de cor" em LEDs aditivos. A seção `[theme]`
controla a calibração aplicada antes de escrever no hardware:

```toml
[theme]
calibrate_led = true      # liga/desliga a calibração
saturation_floor = 1.0    # piso de saturação HSV (1.0 = matiz puro; menor = mais suave)
value_target = 1.0        # brilho alvo (V) enviado ao hardware
```

O **modo "fiel ao tema"** (padrão) mantém o matiz original: o LED fica exatamente
igual ao swatch exibido no popup. Reduza `saturation_floor` para tons mais suaves.

---

## 📁 Estrutura do Projeto

```
omaaura-theme/
├── manifest.json       # Manifest do plugin Omarchy (schemaVersion 1)
├── BarWidget.qml       # Widget da barra do Omarchy (Quickshell)
├── Cargo.toml          # Projeto Rust (binário `omaaura` v2)
├── src/                # Código-fonte Rust (substitui o pacote Python)
│   ├── main.rs         # CLI (clap) + aplicação/daemon/lock
│   ├── hardware.rs     # I2C ENE Aura + OpenRGB (guard anti-conflito de GPU)
│   ├── color.rs        # Calibração de cor para LEDs
│   ├── config.rs       # Config TOML (~/.config/omaaura/config.toml)
│   ├── palette.rs      # Paleta do popup (tema + wallpaper)
│   └── setup.rs        # Assistente de configuração de hardware
├── bin/
│   ├── omaaura         # Wrapper: executa o binário Rust compilado
│   └── omaaura-theme   # Wrapper de compatibilidade -> bin/omaaura
├── service/
│   └── omaaura.service # Unidade systemd --user (embutida no binário)
├── hooks/
│   └── theme-set       # Hook para o evento omarchy hook theme-set
├── install.sh          # Build (cargo) + instalação e configuração automática
├── LICENSE             # Licença MIT
└── README.md           # Documentação
```

---

## 📄 Licença

Distribuído sob a licença MIT. Veja `LICENSE` para mais detalhes.
