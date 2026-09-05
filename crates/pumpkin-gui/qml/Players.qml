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
                Layout.fillWidth: true
                Layout.maximumWidth: 280
                placeholderText: qsTr("Search players…")
                onTextChanged: page.filter = text
            }

            Text {
                text: page.filter === "" ? qsTr("%1 online").arg(rows.count) : qsTr("%1 of %2").arg(rows.count).arg(page.players.length)
                color: Theme.fgMuted
                font.pixelSize: 12
            }

            Item {
                Layout.fillWidth: true
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
