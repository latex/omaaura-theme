# omaaura-theme

Plugin para o [Omarchy Linux](https://omarchy.org/) que sincroniza automaticamente as cores dos LEDs de hardware (ASUS Aura, placas-mãe ROG/TUF e placas de vídeo GeForce RTX) com o tema visual ativo da área de trabalho.

---

## 💡 Recursos

- **Sincronização com o Tema:** Sempre que você troca o tema (`omarchy theme set`), a cor de destaque (`accent`) é imediatamente aplicada aos LEDs do hardware em segundo plano.
- **Detecção Inteligente do Wallpaper:** Extrai automaticamente a cor predominante e os destaques vibrantes do papel de parede ativo (`~/.local/state/omarchy/current/background`). Trocar o wallpaper atualiza a paleta na hora.
- **Menu Popup Interativo na Barra (`PopupCard`):**
  - **Clique Esquerdo no Ícone:** Abre o seletor com as cores do tema Omarchy e as cores dominantes do wallpaper atual.
  - **Clique em Qualquer Cor:** Aplica imediatamente a cor escolhida diretamente nos LEDs de hardware (GPU TUF RTX e Placa-mãe ROG B550-F + Fans ARGB).
  - **Clique Direito no Ícone:** Liga / Desliga a iluminação sem abrir menus.
- **CLI Integrada:** Utilitário `bin/omaaura-theme` para controle via terminal e scripts.
- **Compatível com OpenRGB:** Suporta motherboards ASUS Aura, GPUs, memórias RAM e headers ARGB.


---

## 🛠️ Requisitos

- **Omarchy Linux** (com `omarchy-shell` / Quickshell).
- **OpenRGB** (`omarchy pkg add openrgb`).

---

## 🚀 Instalação

### Instalação Rápida (via repositório)
```bash
./install.sh
```

### Instalação via Omarchy Plugin Manager
```bash
omarchy plugin add https://github.com/seu-usuario/omaaura-theme.git --enable --yes
```

---

## 💻 Uso via Linha de Comando (CLI)

O script `bin/omaaura-theme` pode ser executado diretamente:

```bash
# Sincroniza com a cor do tema Omarchy atual
bin/omaaura-theme sync

# Alterna entre ligado e desligado
bin/omaaura-theme toggle

# Desliga os LEDs
bin/omaaura-theme off

# Aplica uma cor hexadecimal específica
bin/omaaura-theme set ff007f

# Mostra o status atual
bin/omaaura-theme status
```

---

## 📁 Estrutura do Projeto

```
omaaura-theme/
├── manifest.json       # Manifest do plugin Omarchy (schemaVersion 1)
├── BarWidget.qml       # Widget da barra do Omarchy (Quickshell)
├── bin/
│   └── omaaura-theme   # CLI e backend de controle do OpenRGB
├── hooks/
│   └── theme-set       # Hook para o evento omarchy hook theme-set
├── install.sh          # Script de instalação e configuração automática
├── LICENSE             # Licença MIT
└── README.md           # Documentação
```

---

## 📄 Licença

Distribuído sob a licença MIT. Veja `LICENSE` para mais detalhes.
