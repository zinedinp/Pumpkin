import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.pumpkin.gui

Item {
    id: page

    // The `Players` QObject: supplies the rows and receives the actions.
    required property var controller

    readonly property var players: controller.rows

    // Binding a ListView straight to the Rust list would rebuild every delegate whenever any
    // field changes -- ping alone moves constantly -- throwing away scroll position and hover
    // state. Syncing into a ListModel row by row keeps the delegates alive.
    ListModel {
        id: rows
    }

    property string filter: ""

    function matches(entry) {
        return page.filter === "" || entry.name.toLowerCase().includes(page.filter.toLowerCase());
    }

    ListModel {
        id: dimCounts
    }

    function rebuildDims() {
        const counts = {};
        const source = page.players;
        for (let i = 0; i < source.length; ++i) {
            const dim = String(source[i].dimension || "").replace("minecraft:", "");
            counts[dim] = (counts[dim] || 0) + 1;
        }

        const keys = Object.keys(counts).sort();
        for (let i = 0; i < keys.length; ++i) {
            const entry = {
                "label": keys[i],
                "count": counts[keys[i]]
            };
            if (i < dimCounts.count)
                dimCounts.set(i, entry);
            else
                dimCounts.append(entry);
        }
        if (dimCounts.count > keys.length)
            dimCounts.remove(keys.length, dimCounts.count - keys.length);
    }

    function sync() {
        const source = page.players.filter(matches);

        for (let i = 0; i < source.length; ++i) {
            const entry = source[i];
            if (i < rows.count) {
                // set() only emits changes for the keys that actually differ.
                rows.set(i, entry);
            } else {
                rows.append(entry);
            }
        }

        if (rows.count > source.length)
            rows.remove(source.length, rows.count - source.length);

        page.rebuildDims();
    }

    function addWhitelist() {
        const name = whitelistField.text.trim();
        if (name === "")
            return;
        page.controller.whitelist(name);
        whitelistField.clear();
    }

    Connections {
        target: page.players !== undefined ? page : null
        function onPlayersChanged() {
            page.sync();
        }
        function onFilterChanged() {
            page.sync();
        }
    }

    Component.onCompleted: sync()

    ReasonDialog {
        id: reasonDialog
        onConfirmed: (action, playerName, reason) => {
            if (action === "ban")
                page.controller.ban(playerName, reason);
            else
                page.controller.kick(playerName, reason);
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.gap

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            ThemedField {
                id: search
                Layout.preferredHeight: Theme.controlHeight
                Layout.preferredWidth: 280
                placeholderText: qsTr("Search players…")
                onTextChanged: page.filter = text
            }

            Repeater {
                model: dimCounts

                delegate: Rectangle {
                    required property string label
                    required property int count

                    implicitHeight: Theme.controlHeight
                    implicitWidth: chipRow.implicitWidth + 16
                    radius: 6
                    color: Theme.surfaceAlt
                    border.color: Theme.border
                    border.width: 1

                    Row {
                        id: chipRow
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: count
                            color: Theme.fg
                            font.pixelSize: 13
                            font.bold: true
                            font.family: "monospace"
                        }
                        Text {
                            text: label
                            color: Theme.fg
                            font.pixelSize: 12
                        }
                    }
                }
            }

            Item {
                Layout.fillWidth: true
            }

            ThemedField {
                id: whitelistField
                Layout.preferredHeight: Theme.controlHeight
                Layout.preferredWidth: 220
                enabled: page.controller.hasCommands
                placeholderText: qsTr("Player name…")
                onAccepted: page.addWhitelist()
            }

            ThemedButton {
                text: qsTr("Whitelist")
                accent: Theme.accent
                Layout.preferredHeight: Theme.controlHeight
                enabled: page.controller.hasCommands && whitelistField.text.trim() !== ""
                onClicked: page.addWhitelist()
            }
        }

        Card {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // Header row
                RowLayout {
                    Layout.fillWidth: true
                    Layout.bottomMargin: 6
                    spacing: 12

                    Text {
                        text: qsTr("NAME")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.preferredWidth: 150
                    }
                    Text {
                        text: qsTr("PING")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.preferredWidth: 60
                        horizontalAlignment: Text.AlignRight
                    }
                    Text {
                        text: qsTr("DIMENSION")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.preferredWidth: 160
                    }
                    Text {
                        text: qsTr("MODE")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.preferredWidth: 80
                    }
                    Text {
                        text: qsTr("ONLINE")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.preferredWidth: 70
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    Text {
                        text: qsTr("ACTIONS")
                        color: Theme.fgMuted
                        font.pixelSize: 10
                        font.bold: true
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 1
                    color: Theme.border
                }

                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: rows
                    boundsBehavior: Flickable.StopAtBounds

                    ScrollBar.vertical: ScrollBar {}

                    delegate: PlayerRowDelegate {
                        width: ListView.view.width
                        controller: page.controller
                        onReasonRequested: (action, playerName) => reasonDialog.open(action, playerName)
                    }
                }

                Text {
                    Layout.fillWidth: true
                    Layout.topMargin: 12
                    visible: rows.count === 0
                    text: page.players.length === 0 ? qsTr("No players online.") : qsTr("No player matches “%1”.").arg(page.filter)
                    color: Theme.fgMuted
                    font.pixelSize: 12
                    horizontalAlignment: Text.AlignHCenter
                }
            }
        }
    }
}
