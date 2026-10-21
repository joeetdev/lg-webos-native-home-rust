# Verificación y límites

## Dispositivo de referencia

LG50UQ8050PSB, webOS7.6.0/webOS22, firmware04.60.90, aarch64 y Homebrew Channel0.7.3. La instalación se probó durante octubre de2026. Los datos de red, cuentas, claves, journals vivos y dumps se mantienen fuera del repositorio.

## Observado en la TV

- Ocho overrides readonly y hashes/ownership válidos del mismo Home, sin otra app/launcher.
- Compacto once apps+Todas12, ampliado14LaunchPoints permitidos, editor/Back/ThinQ y herramientas finales.
- Reloj libre gris azulado, segundos arriba, mitad de tamaño principal anterior. Comparación de límites tipográficos estable mientras cambiaban segundos.
- Gate final probado en tres reinicios por software sin abrir Homebrew, incluidos dos lanzamientos anticipados forzados de Home viejo. Ventana sin marca retenida; sólo la personalizada verificada liberada. Prueba histórica del gate, no un coldboot eléctrico de la última revisión del mando.
- Captura física confirmó GoogleVoice18874398 y USP18874413 con press/release y deviceId3. El usuario comprobó toggle, controles básicos, MOVIES→Crunchyroll y mantenimiento del bloqueo al abrir la app.
- La primera prueba con restauración automática de ocho segundos era temporal y fue sustituida por toggle sin caducidad.
- La primera política de powerState indicaba bloqueado, pero el cursor reaparecía físicamente al encender. Tras añadir observación de visibilidad nativa, el usuario repitió apagado/encendido con mando, dejando antes ON: volvió OFF y el toggle funcionó.
- Instalación permanente del mando, desactivación, reaplicación e idempotencia verificadas. Modelo, handler y QObject final cargados; `desiredEnabled=false`, expiry0 y Timer de restitución inactivo. Core/QML idénticos a la prueba física; loader final sin trazas.
- Cadena final de referencia:17handlers LG+gateHome+remapeo. Versiones de134apps, precargas persistentes, hook de Home y rollback conservados durante la fase del mando. Diagnósticos/temporales retirados después de guardar evidencia privada.

## Pruebas offline públicas

`python tools/test.py` ejecuta las suites incluidas sin acceder a una TV. Los fixtures de gate/runtime son sintéticos; no son registros de una TV ni una afirmación de aceptación física. La reconstrucción exacta de los ocho fuentes se verificó localmente contra originales privados; el repositorio sólo guarda hashes y resumen saneado, no el firmware.

Los tests históricos que dependían de rutas personales fueron adaptados a fixtures locales portables; no se cambió el controlador instalado para ocultar datos. CI y pruebas locales no son lo mismo: este repositorio incluye el ejecutor local, sin afirmar una ejecución de GitHub Actions inexistente.

## No demostrado por esta revisión

- Compatibilidad con otro firmware/modelo/mando.
- Ausencia de todo frame stock bajo cualquier avería; el recovery puede retirar el gate.
- Retirada de corriente/coldboot eléctrico o nuevo reboot completo de la última versión del mando. Apagado/encendido físico observado es standby/resume.
- Ratón USB o touchpad de teléfono con la política global de puntero.
- Nueva reproducción AirPlay/HDMI/DRM, calidad de señal o encendido desde iPhone en esta última fase. Apertura de interfaces no equivale a reproducción.
- Ahorro causal de RAM/CPU. Se prioriza bajo coste y no se añaden servicios residentes, pero las mediciones anteriores no se extrapolan a cargas futuras.

No se publican los backups privados como evidencia descargable; contienen estado específico y materiales que no corresponden a una distribución pública.
