#!/bin/bash
set -e

# -y/--yes: assume "sim" para as ações opt-in (ex.: adicionar ao grupo i2c).
# Sem a flag, o instalador PERGUNTA antes de qualquer ação privilegiada.
OMAURA_ASSUME_YES=0
for _arg in "$@"; do
  case "$_arg" in -y|--yes) OMAURA_ASSUME_YES=1 ;; esac
done
export OMAURA_ASSUME_YES

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ID="io.github.latex.omaaura-theme"
TARGET_PLUGIN_DIR="$HOME/.config/omarchy/plugins/$PLUGIN_ID"

echo "=== Instalando $PLUGIN_ID (OmaAura v2 — Rust) ==="

# 1. Dependência de runtime para a placa-mãe / ARGB (a GPU usa I2C direto).
if ! command -v openrgb >/dev/null 2>&1; then
  echo "Instalando dependência openrgb..."
  omarchy pkg add openrgb
fi

# 2. Compilar o binário Rust.
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo (Rust) é necessário para compilar o OmaAura. Instale com: omarchy pkg add rust" >&2
  exit 1
fi
echo "Compilando o binário (cargo build --release)..."
( cd "$PROJECT_DIR" && cargo build --release )
BIN="$PROJECT_DIR/target/release/omaaura"
if [ ! -x "$BIN" ]; then
  echo "Falha ao compilar o binário: $BIN" >&2
  exit 1
fi

# 3. Copiar arquivos do plugin para ~/.config/omarchy/plugins/
echo "Copiando arquivos do plugin para $TARGET_PLUGIN_DIR..."
mkdir -p "$TARGET_PLUGIN_DIR/bin"
cp "$PROJECT_DIR"/manifest.json "$PROJECT_DIR"/BarWidget.qml "$PROJECT_DIR"/README.md "$PROJECT_DIR"/LICENSE "$TARGET_PLUGIN_DIR"/
cp "$PROJECT_DIR"/bin/omaaura "$PROJECT_DIR"/bin/omaaura-theme "$TARGET_PLUGIN_DIR"/bin/
cp "$BIN" "$TARGET_PLUGIN_DIR/bin/omaaura-bin"
chmod +x "$TARGET_PLUGIN_DIR"/bin/omaaura "$TARGET_PLUGIN_DIR"/bin/omaaura-theme "$TARGET_PLUGIN_DIR"/bin/omaaura-bin

mkdir -p "$HOME/.local/bin"
ln -nsf "$TARGET_PLUGIN_DIR"/bin/omaaura "$HOME/.local/bin/omaaura"
ln -nsf "$TARGET_PLUGIN_DIR"/bin/omaaura-theme "$HOME/.local/bin/omaaura-theme"

# 4. Instalar Hooks do Omarchy
echo "Configurando hooks do Omarchy..."
mkdir -p "$HOME/.config/omarchy/hooks/theme-set.d" "$HOME/.config/omarchy/hooks/post-boot.d"
cp "$PROJECT_DIR"/hooks/theme-set "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh"
chmod +x "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh"
ln -nsf "$HOME/.config/omarchy/hooks/theme-set.d/omaaura-theme.sh" "$HOME/.config/omarchy/hooks/post-boot.d/omaaura-theme.sh"

# 5. Validar plugin
echo "Validando plugin..."
omarchy plugin validate "$TARGET_PLUGIN_DIR" || true

# 6. Ativar plugin e recarregar o shell
#    O Omarchy desliga o file-watcher do Quickshell de propósito: trocar os
#    arquivos do plugin NÃO recarrega o QML em execução. Só um restart do shell
#    aplica a mudança (o antigo `rescanPlugins` não existe e era no-op).
echo "Ativando plugin $PLUGIN_ID..."
omarchy plugin enable "$PLUGIN_ID" --section right || true
echo "Reiniciando o shell para carregar o novo QML..."
omarchy restart shell || echo "Aviso: rode 'omarchy restart shell' manualmente se o widget não atualizar."

# 7. Acesso I2C (GPU ASUS via ENE Aura) — requer o grupo 'i2c'.
#    Sem isso, o backend ene_i2c não escreve na GPU. Ação privilegiada e
#    OPT-IN: só roda com consentimento explícito e é idempotente.
ensure_i2c_group() {
  if ! compgen -G '/dev/i2c-*' >/dev/null 2>&1; then
    echo "ℹ Nenhum /dev/i2c-* encontrado (o módulo i2c-dev carrega ao detectar o hardware)."
    return 0
  fi
  if id -nG "$USER" 2>/dev/null | tr ' ' '\n' | grep -qx i2c; then
    echo "✔ Grupo 'i2c' já configurado."
    return 0
  fi
  echo "⚠ O controle da GPU via I2C requer o grupo 'i2c'."
  if [ -t 0 ] && [ "${OMAURA_ASSUME_YES:-0}" != "1" ]; then
    printf "  Adicionar '%s' ao grupo 'i2c' agora (sudo usermod -aG i2c)? [y/N]: " "$USER"
    read -r _ans
    case "$_ans" in
      [yYsS]*) ;;
      *) echo "  Pulado. Para habilitar depois: sudo usermod -aG i2c $USER"; return 0 ;;
    esac
  elif [ "${OMAURA_ASSUME_YES:-0}" != "1" ]; then
    echo "  Sem TTY. Para habilitar: sudo usermod -aG i2c $USER"
    return 0
  fi
  if sudo usermod -aG i2c "$USER"; then
    echo "✔ '$USER' adicionado ao grupo 'i2c'. Faça logout/login para o grupo valer."
  else
    echo "⚠ Falha ao adicionar o grupo. Rode manualmente: sudo usermod -aG i2c $USER"
  fi
}
ensure_i2c_group

# 8. Configuração de hardware + serviço (setup NÃO-interativo).
#    Sem isto o widget abre, mas não controla LEDs: faltariam
#    ~/.config/omaaura/config.toml e o serviço systemd --user.
echo "Configurando hardware e serviço (omaaura setup -y)..."
"$HOME/.local/bin/omaaura" setup -y || echo "⚠ 'omaaura setup -y' falhou; rode 'omaaura setup' manualmente."

# 9. Sincronização inicial
echo "Sincronizando iluminação inicial..."
"$HOME/.local/bin/omaaura-theme" sync || true

echo "=== Instalação concluída com sucesso (OmaAura v2)! ==="
