import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.pumpkin.gui

Item {
    id: page

    required property var stats

    readonly property var worlds: stats.worlds

    // Same ListModel sync as the player table: world time ticks every sample, and rebuilding
    // the delegates would throw away scroll position.
    ListModel {
        id: rows
    }

    function sync() {
        const source = page.worlds;

        for (let i = 0; i < source.length; ++i) {
            const entry = source[i];
            if (i < rows.count)
                rows.set(i, entry);
            else
                rows.append(entry);
        }

        if (rows.count > source.length)
            rows.remove(source.length, rows.count - source.length);
    }

    function weatherLabel(weather) {
        switch (weather) {
        case "rain":
            return qsTr("Rain");
        case "thunder":
            return qsTr("Thunder");
        case "clear":
            return qsTr("Clear");
        default:
            return weather;
        }
    }

    Connections {
        target: page.stats
        function onWorldsChanged() {
            page.sync();
        }
    }

    Component.onCompleted: sync()

    Card {
        anchors.fill: parent

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.bottomMargin: 6
                spacing: 12

                Text {
                    text: qsTr("NAME")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 140
                }
                Text {
                    text: qsTr("DIMENSION")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 160
                }
                Text {
                    text: qsTr("CHUNKS")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 70
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    text: qsTr("ENTITIES")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 70
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    text: qsTr("TIME")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 50
                }
                Text {
                    text: qsTr("WEATHER")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    Layout.preferredWidth: 80
                }
                Item {
                    Layout.fillWidth: true
                }
                Text {
                    text: qsTr("SIZE")
                    color: Theme.fgMuted
                    font.pixelSize: 10
                    font.bold: true
                    horizontalAlignment: Text.AlignRight
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

                delegate: Rectangle {
                    required property string name
                    required property string dimension
                    required property int chunks
                    required property int entities
                    required property real timeOfDay
                    required property string weather
                    required property real size

                    width: ListView.view.width
                    height: 34
                    color: hover.hovered ? Theme.surfaceAlt : "transparent"

                    HoverHandler {
                        id: hover
                    }

                    RowLayout {
                        anchors.fill: parent
                        spacing: 12

                        Text {
                            text: name
                            color: Theme.fg
                            font.pixelSize: 12
                            elide: Text.ElideRight
                            Layout.preferredWidth: 140
                        }
                        Text {
                            text: dimension.replace("minecraft:", "")
                            color: Theme.fgMuted
                            font.pixelSize: 12
                            elide: Text.ElideRight
                            Layout.preferredWidth: 160
                        }
                        Text {
                            text: chunks
                            color: Theme.fg
                            font.pixelSize: 12
                            font.family: "monospace"
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 70
                        }
                        Text {
                            text: entities
                            color: Theme.fg
                            font.pixelSize: 12
                            font.family: "monospace"
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 70
                        }
                        Text {
                            text: Format.gameTime(timeOfDay)
                            color: Theme.fgMuted
                            font.pixelSize: 12
                            font.family: "monospace"
                            Layout.preferredWidth: 50
                        }
                        Text {
                            text: page.weatherLabel(weather)
                            color: Theme.fgMuted
                            font.pixelSize: 12
                            Layout.preferredWidth: 80
                        }
                        Item {
                            Layout.fillWidth: true
                        }
                        Text {
                            text: Format.bytes(size)
                            color: Theme.fg
                            font.pixelSize: 12
                            font.family: "monospace"
                            horizontalAlignment: Text.AlignRight
                        }
                    }

                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        height: 1
                        color: Theme.border
                        opacity: 0.5
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                Layout.topMargin: 12
                visible: rows.count === 0
                text: qsTr("No worlds loaded.")
                color: Theme.fgMuted
                font.pixelSize: 12
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
