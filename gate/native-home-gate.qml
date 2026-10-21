import QtQuick 2.4
import WebOSCoreCompositor 1.0
import "gate-proof.js" as Proof

QtObject {
    id: gate
    property bool ready: false
    property var held: []
    property int attempts: 0

    property WindowModel observer: WindowModel {
        surfaceSource: compositor.surfaceModel
        acceptFunction: "accept"
        function accept(item) { return item && item.appId === "com.webos.app.home"; }
        onSurfaceAdded: gate.registerSurface(item)
        onCountChanged: {
            for (var i = 0; i < count; i++) gate.registerSurface(get(i));
        }
        onInvalidated: {
            for (var i = 0; i < count; i++) gate.registerSurface(get(i));
        }
    }

    property Timer poller: Timer {
        interval: 500
        repeat: true
        running: !gate.ready && gate.attempts < 240
        onTriggered: { gate.attempts++; gate.refresh(); }
    }

    Component.onCompleted: {
        for (var i = 0; i < gate.observer.count; i++) gate.registerSurface(gate.observer.get(i));
        gate.refresh();
    }
    Component.onDestruction: gate.releaseAll()

    function readText(path) {
        var request = new XMLHttpRequest();
        request.open("GET", "file://" + path, false);
        request.send();
        if (request.readyState !== XMLHttpRequest.DONE ||
            (request.status !== 0 && request.status !== 200) || !request.responseText)
            throw new Error("Native Home readiness file is unavailable");
        return request.responseText;
    }

    function refresh() {
        gate.ready = Proof.nativeGateProof(gate.readText);
    }

    function registerSurface(item) {
        if (!item || item.appId !== "com.webos.app.home") return;
        for (var i = 0; i < gate.held.length; i++)
            if (gate.held[i].item === item) return;
        var saved = {item: item, opacity: item.opacity};
        gate.held.push(saved);
        item.opacity = Qt.binding(function() {
            return gate.ready && item.windowProperties &&
                   item.windowProperties.lgNativeHomeReady === "lg-native-home-v1" ? saved.opacity : 0;
        });
        gate.attempts = 0;
        gate.refresh();
    }

    function releaseAll() {
        for (var i = 0; i < gate.held.length; i++) {
            try { gate.held[i].item.opacity = gate.held[i].opacity; } catch (e) {}
        }
        gate.held = [];
    }
}
