import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "omaaura-theme"

  readonly property string home: Quickshell.env("HOME")
  readonly property string scriptPath: {
    var localBin = decodeURIComponent(String(Qt.resolvedUrl("bin/omaaura-theme")).replace(/^file:\/\//, ""))
    return localBin
  }

  property string themeColor: "#89b4fa"
  property string rgbState: "on"
  property bool working: false

  readonly property bool showLabel: setting("showLabel", false) === true
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property string tooltipText: "OmaAura: " + themeColor + " (" + (rgbState === "off" ? "Desligado" : "Ativo") + ")\nClique Esq: Sincronizar | Clique Dir: Ligar/Desligar"

  onTooltipTextChanged: if (mouseArea.containsMouse && bar) bar.showTooltip(root, tooltipText)

  function sync() {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "sync"]
    syncProcess.running = true
  }

  function toggle() {
    if (working) return
    working = true
    syncProcess.command = [root.scriptPath, "toggle"]
    syncProcess.running = true
  }

  implicitWidth: content.implicitWidth + Style.space(10)
  implicitHeight: barSize

  FileView {
    path: root.home + "/.local/state/omarchy/current/theme/keyboard.rgb"
    watchChanges: true
    printErrors: false
    onLoaded: {
      var raw = text().trim()
      if (raw.length > 0) {
        if (raw.indexOf("#") !== 0) raw = "#" + raw
        if (raw.length === 7) {
          root.themeColor = raw
        }
      }
    }
    onFileChanged: reload()
  }

  FileView {
    path: root.home + "/.local/state/omaaura-theme/state"
    watchChanges: true
    printErrors: false
    onLoaded: {
      var raw = text().trim()
      if (raw.indexOf("off") === 0) {
        root.rgbState = "off"
      } else {
        root.rgbState = "on"
      }
    }
    onFileChanged: reload()
  }

  Process {
    id: syncProcess
    command: []
    onExited: root.working = false
  }

  Row {
    id: content
    anchors.centerIn: parent
    spacing: Style.space(6)

    OpticalGlyph {
      anchors.verticalCenter: parent.verticalCenter
      width: Style.bar.iconSlot
      height: Style.bar.iconSlot
      text: root.rgbState === "off" ? "󰌶" : "󰌵"
      fontFamily: root.fontFamily
      fontSize: Style.font.icon
      color: root.rgbState === "off" ? Qt.darker(root.foreground, 1.8) : root.themeColor
      opacity: root.working ? 0.4 : 1.0

      Behavior on opacity { NumberAnimation { duration: 150 } }
      Behavior on color { ColorAnimation { duration: 250 } }
    }

    Text {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.showLabel && !root.vertical
      text: root.themeColor
      color: root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
      renderType: Text.NativeRendering
    }
  }

  MouseArea {
    id: mouseArea
    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton
    cursorShape: Qt.PointingHandCursor
    onClicked: function(mouse) {
      if (mouse.button === Qt.RightButton) {
        root.toggle()
      } else {
        root.sync()
      }
    }
    onEntered: if (root.bar) root.bar.showTooltip(root, root.tooltipText)
    onExited: if (root.bar) root.bar.hideTooltip(root)
  }
}
