---
title: Estado
description: Qué está hecho, qué está en curso y cómo se hace realmente un cambio a una decisión establecida en este proyecto.
date: 2026-08-22
lang: es
---

*[English](../status.html) · **Español***

# Estado

## Hitos

| Hito | Estado |
|------|--------|
| Andamiaje, CI, instrumentos de prueba | hecho |
| Motor de cinta: transporte, grabación, pinchado, deshacer, persistencia | hecho |
| DSP lo-fi y pérdida de generación | hecho |
| Mezcla, mixdown, exportación WAV/MP3/MP4, CLI | hecho |
| Audio en tiempo real (cpal) | verificado en macOS y en hardware Raspberry Pi |
| Interfaz Slint: transporte, tiras de pista (armar/silenciar/monitor/fader/paneo), medidores, barra de posición de cinta, guardar/deshacer, gestión de casetes, exportación, audio real | hecho |
| Despliegue en Raspberry Pi | en curso, ver abajo |
| Bus de mezcla estéreo (cambio 001) | hecho, publicado en v0.1.0 |

El hito de Raspberry Pi ya cubre bastante terreno: compilación
aarch64, la capa de dispositivos ALSA/PipeWire, grabación y guardado
full-duplex, reconexión automática al dispositivo recordado, arranque
automático en modo kiosco con íconos de barra de tareas y escritorio,
teclado en pantalla y autoguardado al detener, todo verificado en
hardware real. Falta una medición formal del tiempo de callback: mirá
[Configuración de Raspberry Pi](raspberry-pi.html) para el detalle.

## Errores reales, encontrados y corregidos

Se encontraron y corrigieron acá tres violaciones distintas de la
propia regla de tiempo real del proyecto - nunca, jamás, reservar
memoria en el hilo del callback de audio - y ninguna salió a la luz por
una falla:

- Grabar reservaba brevemente un buffer del tamaño de toda la cinta
  restante en ese hilo. Corregido capturando el audio desplazado en
  bloques chicos pre-reservados.
- Un camino de expulsión soltaba esos bloques de vuelta al heap en
  lugar de devolverlos a la reserva, lo que además de liberar memoria
  en el hilo de audio iba agotando la reserva con el tiempo.
- Enganchar la grabación reconstruía la cadena de DSP entera desde
  cero - cuatro o cinco reservas de memoria - cada vez. Ese venía
  publicado sin que nadie lo notara desde hacía meses.

El segundo y el tercero los encontraron revisiones adversariales de una
propuesta que no tenía nada que ver con ellos. Cada uno se corrigió con
una prueba de regresión que falla contra el comportamiento viejo.

La regla tampoco se argumenta ya de forma estructural. Un asignador
global contador, solo para pruebas, mide ahora el camino de tiempo real
directamente, y la primera vez que corrió encontró cuatro reservas más
que el razonamiento cuidadoso había pasado por alto: un objeto de
pasada reconstruido en cada toma, y tres lugares distintos donde ceder
un contenedor perdía la capacidad que la toma siguiente necesitaba. El
camino está medido hoy en cero reservas y cero liberaciones, tanto para
una pasada de grabación común como para una mezcla.

## Cambiar una decisión establecida

Este proyecto trata su especificación como una constitución: la
cantidad de pistas, el flujo destructivo, la cadena de DSP; nada de eso
se "mejora" casualmente de paso. Revertir o extender una decisión
establecida y visible para el usuario requiere una propuesta escrita y
el visto bueno de una revisión adversarial de la especificación antes
de que empiece cualquier implementación.

La propuesta que reemplazó la mezcla original por un bus de mezcla
estéreo dedicado, impreso en tiempo real, es el ejemplo más claro de
ese proceso funcionando como corresponde y no como un trámite. Corrigió
dos limitaciones reales: la mezcla vieja colapsaba la información
estéreo a mono, y era de un solo uso, así que una segunda mezcla
descartaba silenciosamente la primera.

Le llevó **doce rondas de revisión a lo largo de trece revisiones**
antes de ser aprobada. Todas las rondas menos la última encontraron
algo real: una falla de diseño en una versión temprana del
almacenamiento de destino, un riesgo de reserva de memoria en tiempo
real en cómo una pasada estéreo capturaría los datos para deshacer, una
regla matemáticamente invertida sobre lo que se escucharía mientras se
mezcla, una estimación de memoria residente que hubo que recalcular más
de una vez contra el código tal como se publicó y no como se suponía
que funcionaba, y - dos veces - una prueba especificada en la propuesta
que nunca podría haber pasado tal como estaba escrita. A ninguna se le
dio el visto bueno de trámite.

Ya está completamente implementado y publicado: una pasada estéreo en
tiempo real, deshacer atómico de dos canales, el bus plegando su propio
contenido anterior hacia adelante para que las mezclas se apilen en vez
de reemplazarse, el fader master demostrablemente sin llegar nunca a la
cinta, y una tira de Bus en la interfaz con su propio fader y silencio.
Verificado en una Raspberry Pi con una interfaz real, no solo en
pruebas.

Eso es más lento que simplemente escribir la función. También es
exactamente el intercambio que un proyecto como este debería hacer.
