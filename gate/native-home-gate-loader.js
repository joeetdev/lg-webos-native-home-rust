var nativeGate = null;

function init() {
    var component = Qt.createComponent("file:///var/lib/lg-native/native-home-gate.qml");
    if (component.status !== Component.Ready)
        throw new Error("Native Home gate component could not load: " + component.errorString());
    nativeGate = component.createObject(root);
    if (!nativeGate) throw new Error("Native Home gate controller could not be created");
}

function handleNativeHomeGate(key, pressed, autoRepeat) {
    if (nativeGate && !nativeGate.ready &&
        (key === Qt.Key_Super_L || key === Qt.Key_Meta || key === Qt.Key_Menu))
        return KeyPolicy.Accepted;
    return KeyPolicy.NextPolicy;
}
