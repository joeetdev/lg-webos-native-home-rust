# Despliegue manual y recuperación

Este documento describe el orden y los contratos, no un instalador universal ni una orden para ejecutar todos los pasos. Los scripts de root pueden dejar Home invisible o provocar su relanzamiento. Mantén acceso SSH independiente, originales y backups externos ANTES de cambiar la TV. No se modifica ningún dispositivo al clonar o probar este repositorio.

## 1. Prerrequisitos

- Dispositivo/firmware exactos indicados en README y hashes de los ocho originales conocidos.
- Root previamente configurado, Homebrew Channel compatible, Python3.8+ en la TV y `luna-send` disponible.
- Inventario privado de apps, versiones, keyFilters, hooks, precargas y montajes. No subirlo a Git.
- Backup externo real de los checkpoints derivados de Home, desreferenciando sus páginas enlazadas, antes de usar el helper de limpieza. Un JSON vacío no es un backup.
- Directorio `/var/lib/lg-native` root-owned, privado0700; archivos regulares root-owned sin escritura de grupo/otros; assets/estados privados0600. Revisar también ancestros y symlinks.
- Imagen de fondo propia/con permiso y las rutas exactas del payload reconstruido. No sustituir silenciosamente imágenes, hashes o rutas en un paquete validado.
- Un guard de recuperación independiente ANTES de cada prueba con efectos; no depender de `genericTimer` ni de un canal SSH que podría desaparecer.

## 2. Reconstrucción offline

Obtener originales NO cubiertos por overrides y ejecutar el constructor de `home/README.md`. Verifica que los ocho productos coinciden con los hashes finales de `home/`. No aplicar parches por similitud ni actualizar la allowlist sólo para forzar un firmware distinto.

El manifiesto generado describe rutas de staging en `/var/lib/lg-native` y los targets nativos. Debes copiar a la TV exactamente los archivos que refiere, verificar SHA y permisos y conservar los originales fuera de la TV. Si cambias rutas, genera y valida un manifiesto coherente: no copies uno de un backup ajeno.

## 3. Runtime de Home

`native_home.py` limita los targets, valida propiedad/metadata, registra intención antes del montaje y usa backing copies inmutables. `nativeapply.py` aplica el manifiesto; `rollback.py` restituye sólo montajes demostrablemente propios.

El staging necesita los módulos del runtime, el manifiesto `home-final-manifest.json` generado, fuentes exactas y un `checkpoint-backup.json` privado que documente el respaldo real. Los helpers no crean ese respaldo por ti. La limpieza sólo considera los dos roots derivados de Home, no inputcommon ni CRIU global.

No ejecutar el hook55 hasta preparar la recuperación del gate: la variante publicada `runtime/55-lg-native` y `runtime/rollback.py` incluyen recuperación con `native_gate_control.py`. Nunca instales un wrapper que importa un módulo ausente.

Antes de persistir: aplicar con guard, comprobar ocho mounts readonly/hashes, abrir Home mediante mando, probar TV/entradas/apps y restauración real. Commit de cada trial antes del siguiente. No tocar startup.sh/jumpstart.sh.

## 4. Gate de presentación temprana

Empaquetado en el host:

```sh
cd gate
python pack-gate.py final
```

Genera tres assets inmutables y `packed-final/package.json`. El proof Python de hashes/montajes sigue siendo la autoridad. El proof JS compara los ocho sources/targets/backings, journal y manifiesto; cada ventana exige además `lgNativeHomeReady=lg-native-home-v1` publicado por el MainView personalizado.

Staging manual: assets y paquete final con nombre remoto `gate-package-final.json`, `native_gate_control.py`, `gate_install.py` y `gate_trial_guard.py`. Revisa antes los wrappers `SAFE_HOOK`, `SAFE_ROLLBACK` y `TRIAL_HOOK`: instalar el gate cambia únicamente los wrappers propios55/rollback y añade una recuperación temporal54. No sobrescribir hooks ajenos.

`gate_install.py --install /var/lib/lg-native/gate-package-final.json` valida e instala con guard; `--check` informa readiness y cadena; `--commit` sólo después de verificación final retira token/hook temporal. El helper `native_gate_control.py --disable` retira sólo referencias exactas del gate; `--recover-if-unready` prioriza Home usable ante fallo de integridad.

El gate no está permitido sin payload/marker funcional y recuperación. Verifica arranque real sin abrir Homebrew; configuración persistente por sí sola no prueba primer frame. No borres los originales del firmware.

## 5. Mando, independiente del Home

En el host, desde `remote/`, `python pack-remote.py` genera paquetes trial/final. El final no contiene telemetría y no tiene caducidad de puntero. El paquete trial actual tiene trazas de observación, NO un servidor instalado ni un temporizador de ocho segundos. No lo instales sin construir una observación privada acotada; usa el final después de probar el hardware.

Staging: `remote_controls.py`, los tres assets del paquete final y `remote-final-package.json`. Confirma ID de Crunchyroll y códigos/dispositivo del mando antes de usar este perfil. Instalar con una recuperación externa ya armada:

```sh
python3 -B -E -s /var/lib/lg-native/remote_controls.py --install /var/lib/lg-native/remote-final-package.json
python3 -B -E -s /var/lib/lg-native/remote_controls.py --check
```

El estado `remote-controls-state.json` se crea localmente y queda privado. La entrada se añade antes de `handleCPHotkeys`; preservar todos los filtros previos, incluido el gate. El total19 sólo describe la instalación de referencia, no una lista que deba forzarse.

Pruebas físicas: OFF inicial; ON/OFF con Google; flechas/OK/Atrás/volumen; MOVIES→Crunchyroll; esperar más de15s sin reactivación; dejar ON, apagar/encender con mando y verificar OFF; activar de nuevo. Probar teléfono/USB aparte si son necesarios. Leer de vuelta estado/paquete/configuración y revisar ausencia de diagnósticos antes de retirar el guard externo.

## Recuperación independiente

Sólo mando:

```sh
python3 -B -E -s /var/lib/lg-native/remote_controls.py --disable
```

Sólo gate:

```sh
python3 -B -E -s /var/lib/lg-native/native_gate_control.py --disable
```

Home stock, conservando lo ajeno y desactivando antes su gate mediante el wrapper seguro:

```sh
python3 -B -E -s /var/lib/lg-native/rollback.py
```

No borrar los estados para saltar ownership. No restaurar ciegamente una cadena vieja de filtros, ni copiar journals o backups de otro dispositivo. Conservar evidencia fuera de la TV; retirar sólo temporales enumerados y verificados por hash. Un comando exitoso no sustituye revisar el resultado.
