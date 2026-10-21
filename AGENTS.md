# AGENTS.md

## Alcance

Conservar el MISMO Home nativo `com.webos.app.home`. No crear otro launcher, dashboard, app, servicio residente ni compositor. Dispositivo de referencia: LG 50UQ8050PSB, webOS7.6.0/webOS22, firmware04.60.90. No afirmar compatibilidad universal.

`home/` contiene deltas reproducibles, no firmware completo. `runtime/` controla montajes; `gate/` protege la presentación temprana; `remote/` implementa el mando y su ownership independiente.

## Desarrollo y verificación

Host: Python3.11+ y Node20+. Runtime de TV: Python3.8 compatible.

```sh
python tools/test.py
python tools/security_check.py
```

Usar RED→GREEN: una prueba que falle antes del cambio, implementación mínima, suite completa afectada después. No rebajar expectativas para ocultar regresiones. Las pruebas ordinarias son offline: no conectar ni modificar una TV sin autorización específica.

Separar pruebas sintéticas, análisis estático, QML compilado, eventos inyectados y pruebas físicas. Una API que abre una app no demuestra reproducción. Apagado con mando no equivale a retirada eléctrica ni reinicio por software. No afirmar CI verde si sólo hubo pruebas locales.

## Invariantes del Home

- Ocho targets exactos y hashes conocidos; rechazar firmware desconocido. Originales del dispositivo fuera de Git.
- Conservar LaunchPoints, objetos `doLaunch`, índices fuente, editor, accesibilidad, foco y navegación nativa.
- Cuadrícula plana, seis columnas, once apps y «Todas las apps» en posición12. Orden explícito del README, no frecuencia de uso; herramientas Apps/Media/Web al final.
- ThinQ sólo lateral con objeto nativo; Ajustes último. USB sólo con almacenamiento real, no por existir un puerto físico.
- Reloj sin tarjeta, color `#ADC4CE`, slots fijos por carácter y segundos arriba. No depender de anchos proporcionales. Actualizar sólo con Home visible/activo, fuera de editor/Recents.

## Seguridad y recuperación

No tocar firmware, particiones, cadena root, startup/jumpstart ni ejecutables compartidos. Conservar TV, HDMI, imagen/sonido, CEC/ARC/eARC, AirPlay, HomeKit/ThinQ, red standby, cuentas, emparejamientos, tienda y DRM. No falsear activationflags ni soporte de precarga. La preferencia de referencia es NetflixON/YouTubeOFF; Crunchyroll no estaba soportado por el gestor.

Validar rutas canónicas, permisos, root ownership, archivo regular, enlaces, hashes e identidad del montaje. Guardar intención y ownership ANTES de efectos, y verificar el destino exacto después de escribir. Éxito de SSH/setConfigs no demuestra que el QML cargó.

Assets JS/QML/loader y copias de montaje inmutables con nombre SHA256: Qt puede reutilizar código antiguo si cambia el contenido manteniendo su URL. El gate sólo toca Home; requiere proof de los ocho overrides Y marca de la ventana concreta, nunca sólo archivos correctos.

Preservar filtros ajenos, orden y seguridad/accesibilidad. Leer la cadena VIVA y quitar sólo diccionarios exactos registrados como propios; nunca restaurar una lista completa antigua. Serializar escritores: configd no ofrece compare-and-swap.

Un cambio de keyFilters destruye loaders, cancela genericTimer y suscripciones. genericTimer NO es un watchdog independiente. Toda prueba requiere recuperación que sobreviva al fallo que se prueba. No añadir VNC, inyectores, polling perpetuo ni servidores diagnósticos residentes.

## Mando

GoogleVoice Qt18874398 y USP/MOVIES Qt18874413 se confirmaron físicamente con deviceId3 en el mando de referencia. No son códigos evdev; capturar otros mandos antes de cambiar su perfil. Consumir press/release; ignorar autorepetición para la acción; descartar releases huérfanos/expirados; mantener los demás botones.

`desiredEnabled` es distinto del flag aplicado. OFF al iniciar/reanudar; ON manual sin caducidad. Mantener elección durante salvapantallas. Usar blockPointer/cancelBlockPointer nativos, no endpoints Luna inventados. Ante reset nativo tardío, observar cursorVisibleChanged y reaplicar OFF sólo cuando el cursor se hace visible, la política deseada es OFF y powerState=active. Respetar activación manual y logo de apagado.

MOVIES usa checkLaunch con el ID instalado, sin parámetros que fuercen relanzamiento. El temporizador de ocho segundos fue sólo una prueba histórica, NO el comportamiento permanente.

## Publicación

Excluir claves SSH, tokens, `.env`, contraseñas, nonces operacionales, cuentas, direcciones privadas/MAC/SSID, rutas personales, estados vivos, inventarios completos, ROMs/BIOS, IPK/SO, dumps y respaldos ZIP/TAR. No añadir el directorio privado de backups. Fixtures pequeños, sintéticos y etiquetados; no presentar datos inventados como resultados físicos.

Los productos generados y los originales permanecen ignorados. No publicar firmware LG completo. La captura documenta el resultado; no concede derechos sobre el fondo ni marcas de terceros. No asignar licencias a archivos sin verificar procedencia y elección del titular.

Antes de publicar: pruebas, revisión del diff, análisis de privacidad/secretos y revisión independiente. Repo personal del titular, no organización; no cambiar visibilidad ni propiedad sin autorización. README/documentación deben distinguir automatización disponible, pasos manuales y límites reales.
