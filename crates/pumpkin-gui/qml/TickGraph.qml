import QtQuick
import org.pumpkin.gui

// The server's rolling window of the last 100 tick durations, oldest on the left.
//
// The sampler unrotates the server's circular buffer before handing it over, so index order is
// chronological here.
Item {
    id: graph

    // Tick durations in milliseconds (QList<double> as a JS sequence).
    property var samples: []
    // How long a tick may take at the configured tick rate. The scale only grows past it when
    // ticks actually overrun.
    property real budgetMs: 50
    readonly property real peak: {
        let max = budgetMs;
        for (let i = 0; i < samples.length; ++i)
            max = Math.max(max, samples[i]);
        return max;
    }

    implicitHeight: 90

    Rectangle {
        anchors.fill: parent
        color: Theme.surfaceAlt
        radius: 4
    }

    // The 50 ms budget line: bars crossing it are ticks that missed 20 TPS.
    Rectangle {
        width: parent.width
        height: 1
        color: Theme.border
        y: parent.height * (1 - graph.budgetMs / graph.peak)
        visible: graph.samples.length > 0
    }

    Row {
        anchors.fill: parent
        anchors.margins: 2
        spacing: 1

        Repeater {
            model: graph.samples

            delegate: Rectangle {
                required property real modelData

                width: (graph.width - 4 - (graph.samples.length - 1)) / graph.samples.length
                height: Math.max(1, (graph.height - 4) * Math.min(1, modelData / graph.peak))
                y: graph.height - 4 - height
                color: Theme.loadColor(modelData / graph.budgetMs)
            }
        }
    }

    // Sits on a pill because tall bars reach the top of the plot and would otherwise run
    // straight through the text.
    Rectangle {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 3
        width: peakLabel.width + 10
        height: peakLabel.height + 4
        radius: 3
        color: Theme.surface
        opacity: 0.85

        Text {
            id: peakLabel
            anchors.centerIn: parent
            text: qsTr("peak %1 ms").arg(graph.peak.toFixed(0))
            color: Theme.fgMuted
            font.pixelSize: 10
            font.family: "monospace"
        }
    }
}
