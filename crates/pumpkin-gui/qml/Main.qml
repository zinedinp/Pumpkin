import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.pumpkin.gui

ApplicationWindow {
    id: root

    width: 1180
    height: 780
    minimumWidth: 760
    minimumHeight: 520
    visible: true
    title: qsTr("Pumpkin")
    color: Theme.background

    ServerStats {
        id: stats
        Component.onCompleted: Theme.preference = themePreference
    }

    PlayerList {
        id: playerList
    }

    Console {
        id: consoleController
    }

    DevTools {
        id: dev

        Component.onCompleted: Theme.animations = screenshotPath === ""
    }

    function stopAndClose() {
        consoleController.requestStop();
        root.close();
    }

    onClosing: {
        if (dev.screenshotPath === "")
            consoleController.requestStop();
    }

    // Headless capture for development and CI: render, save a PNG, exit. Off unless
    // PUMPKIN_GUI_SCREENSHOT is set. Grabs `shell`, not the window's contentItem: the latter is
    // created from C++ and has no QML engine to grab with.
    Timer {
        running: dev.screenshotPath !== ""
        interval: dev.screenshotDelayMs
        onTriggered: {
            const grabbed = shell.grabToImage(function (result) {
                result.saveToFile(dev.screenshotPath);
                Qt.quit();
            });
            if (!grabbed)
                Qt.quit();
        }
    }

    // Polling instead of pushing: nothing has to cross a thread boundary into a Qt object.
    Timer {
        interval: 500
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            stats.refresh();
            playerList.refresh();
            consoleController.refresh();
        }
    }

    // A real background rather than relying on the window's own fill: `grabToImage` captures
    // only this item, and anything the item does not paint comes out transparent.
    Rectangle {
        id: shell

        anchors.fill: parent
        color: Theme.background

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // The header lives inside the layout rather than in ApplicationWindow.header so the whole
            // UI is one QML item tree, which is what makes the headless grab above possible.
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 52
                color: Theme.surface

                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: 1
                    color: Theme.rule
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 16
                    anchors.rightMargin: 12
                    spacing: 16

                    Text {
                        text: stats.serverReady ? qsTr("Pumpkin %1").arg(stats.pumpkinVersion) : qsTr("Starting…")
                        color: Theme.fg
                        font.pixelSize: 15
                        font.bold: true
                    }

                    Badge {
                        label: qsTr("TPS")
                        value: stats.tps.toFixed(1)
                        accent: Theme.tpsColor(stats.tps)
                        visible: stats.serverReady
                    }

                    Badge {
                        label: qsTr("PLAYERS")
                        value: stats.playerCount
                        accent: Theme.accent
                        visible: stats.serverReady
                    }

                    Badge {
                        label: qsTr("UPTIME")
                        value: Format.duration(stats.uptimeSecs)
                        accent: Theme.fgMuted
                        visible: stats.serverReady
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    // Centred between the status badges and the theme toggle: it is the one
                    // action in the header, and it should not sit next to a harmless toggle.
                    ThemedButton {
                        text: qsTr("Stop Server")
                        accent: Theme.danger
                        enabled: consoleController.hasCommands
                        onClicked: root.stopAndClose()
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    ToolButton {
                        text: Theme.dark ? "☀" : "☾"
                        font.pixelSize: 16
                        onClicked: Theme.toggle()
                        ToolTip.visible: hovered
                        ToolTip.text: Theme.dark ? qsTr("Light theme") : qsTr("Dark theme")
                    }
                }
            }

            TabBar {
                id: tabs
                Layout.fillWidth: true
                currentIndex: dev.initialTab

                background: Rectangle {
                    color: Theme.surface

                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        height: 1
                        color: Theme.rule
                    }
                }

                ThemedTabButton {
                    text: qsTr("Overview")
                }
                ThemedTabButton {
                    text: qsTr("Performance")
                }
                ThemedTabButton {
                    text: qsTr("Worlds")
                }
                ThemedTabButton {
                    text: qsTr("Players")
                }
            }

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.margins: Theme.gap
                currentIndex: tabs.currentIndex

                Overview {
                    stats: stats
                    consoleController: consoleController
                }
                Performance {
                    stats: stats
                }
                Worlds {
                    stats: stats
                }
                Players {
                    controller: playerList
                }
            }
        }
    }
}
