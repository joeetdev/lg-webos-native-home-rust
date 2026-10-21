# Arquitectura

```text
Firmware original de la propia TV (no se distribuye)
  -> deltas Home con hashes exactos
  -> ocho fuentes generadas en staging privado
  -> runtime root: ownership + backing copies + bind mounts readonly
  -> MISMO com.webos.app.home / LaunchPoints y acciones LG

Compositor existente
  -> cadena LG de keyFilters, conservada en orden
     -> gate no visual antes de handleSystemKeys
        -> proof ocho overrides + marca por ventana -> libera opacidad guardada
     -> remapeo antes de handleCPHotkeys
        -> Google: política nativa blockPointer / cancelBlockPointer
        -> MOVIES: applicationManager.checkLaunch Crunchyroll
```

## Límites de autoridad

El runtime Python comprueba SHA, metadata, inodes y ownership real de los mounts; el proof JS no sustituye esas validaciones. El gate afecta sólo a ventanas Home. Un Home antiguo sin marker permanece retenido aunque los archivos actuales sean correctos.

Los estados de runtime, gate y mando pertenecen al dispositivo y NO forman parte del repo. Cada subsistema retira sólo sus entradas/montajes exactos, preservando lo ajeno. Sus helpers son one-shot; no añaden procesos residentes.

## Ciclo del puntero

`desiredEnabled=false` inicial y tras standby real. Mientras active, Google alterna la política; un salvapantallas preserva la elección manual. El sistema puede restituir soporte nativo tarde al volver de standby: observar `cursorVisibleChanged` corrige únicamente el caso visible+desiredOFF+active, sin polling continuo y sin combatir ON manual ni el logo de apagado.

El controlador no visual tiene un cleanup asociado al loader, que habilita y destruye su instancia al retirarlo. No se inyecta código en lginput ni se usan endpoints de cursor ficticios. El alcance global de la API exige probar dispositivos de puntero distintos por separado.

## Activos y caché

JS, QML y loader se nombran por SHA256, encadenando las referencias antes de calcular el hash siguiente. Reescribir un archivo con la misma URL puede reutilizar una compilación anterior del motor Qt. No reiniciar el compositor compartido sólo para borrar esa caché.
