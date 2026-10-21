function createRemoteControls(env) {
    var pressed = {};
    function reset() { pressed = {}; }
    function handle(key, down, repeat, device) {
        if ((key !== 18874398 && key !== 18874413) || device !== env.device) return env.next;
        if (!env.active()) { reset(); return env.accepted; }
        if (repeat) return env.accepted;
        if (down) { if (pressed[key] === undefined) pressed[key] = env.now(); }
        else if (pressed[key] !== undefined) {
            var started = pressed[key];
            delete pressed[key];
            var elapsed = env.now() - started;
            if (elapsed >= 0 && elapsed <= 10000) {
                if (key === 18874398) env.toggle(); else env.movies();
            }
        }
        return env.accepted;
    }
    return {handle:handle, reset:reset};
}
