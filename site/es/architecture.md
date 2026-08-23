---
title: Cómo está construido
description: Distribución de crates, la cadena de DSP, seguridad en tiempo real y cómo se prueba la corrección del audio sin alguien escuchando.
date: 2026-08-22
lang: es
---

*[English](../architecture.html) · **Español***

# Cómo está construido

## Los crates

```
crates/porta-dsp/      carácter de cinta: saturación, ancho de banda, flutter, siseo
crates/porta-engine/   cinta, transporte, pasadas de grabación, deshacer, mezclador, proyectos
crates/porta-testkit/  instrumentos de prueba: generadores, medidores, FFT, detector de clics
crates/porta-app/      CLI, guiones de sesión, exportación WAV, adaptador de tiempo real, interfaz Slint
```

La dirección de las dependencias va siempre en un solo sentido:
`porta-dsp` no sabe nada de lo que está por encima, `porta-engine`
depende de `porta-dsp` pero no sabe nada de hardware ni de interfaz, y
`porta-app` es el único crate al que se le permite saber que existen
cpal o Slint. La API del motor son buffers que entran y buffers que
salen: no hay noción de placa de sonido, ventana ni hilo en ningún
lugar dentro de él.

No es un ejercicio de abstracción por sí mismo. Es lo que hace que todo
el motor sea comprobable sin placa de sonido en CI, y lo que hizo que
[la compilación para Raspberry Pi](raspberry-pi.html) funcionara la
primera vez que corrió contra hardware real: el motor mismo nunca notó
que había cambiado de plataforma.

## El camino de grabación

Una "pasada de grabación" es la unidad tanto de grabación como de
deshacer: un enganche continuo sobre una pista armada, desde el
pinchado de entrada hasta el de salida. Antes de que se sobrescriba
cualquier muestra, se captura el contenido de cinta que desplaza, para
que deshacer pueda restaurar la región byte a byte, y el costo de esa
captura es proporcional a lo que realmente se grabó, no al largo de la
cinta.

Cada pasada atraviesa la cadena de carácter completa antes de
cuantizar a 16 bits, en este orden: saturación (tanh, con ganancia de
entrada y compensación), limitación de ancho de banda (un pasa-bajos
cerca de 11kHz, un pasa-altos cerca de 60Hz), wow y flutter (un retardo
fraccionario modulado, re-sembrado en cada pasada para que las
generaciones sucesivas no compartan una única fluctuación coherente),
siseo (ruido filtrado y sembrado, impreso dentro de la banda de paso) y
una etapa opcional de reducción de bits, desactivada por defecto. El
dither TPDF se aplica al final, inmediatamente antes de cuantizar. Nada
de esto es cosmético: es lo que hace que tres generaciones de una
mezcla sean medible - no solo plausiblemente - más apagadas y ruidosas
que dos, que es la prueba de aceptación real que la cadena de DSP tiene
que pasar.

## Seguridad en tiempo real

El callback de audio - la función que cpal llama en su propio hilo de
tiempo real - tiene una regla innegociable: nunca, jamás, reservar
memoria, tomar locks ni hacer E/S de disco en ese hilo. Los mensajes de
control (armar, faders, comandos de transporte) cruzan una cola sin
esperas en lugar de tocar el motor directamente.

No es una regla que vive solo en un documento. Ha sido el origen de
varios errores reales encontrados y corregidos en este proyecto: grabar
reservaba brevemente un buffer del tamaño de toda la cinta restante en
ese hilo (corregido capturando el audio desplazado en bloques chicos
pre-reservados en lugar de una gran reserva bajo demanda); se descubrió
que un camino de expulsión soltaba esos bloques de vuelta al heap en
lugar de devolverlos a la reserva; y enganchar la grabación
reconstruía la cadena de DSP entera desde cero - cuatro o cinco
reservas de memoria - cada vez, algo que nadie había notado en meses.
Cada uno se encontró por revisión adversarial y no por una falla, y
cada uno se corrigió igual: con una prueba de regresión que falla
contra el comportamiento viejo.

## Probar sin alguien escuchando

La corrección del audio se verifica renderizando sin tiempo real y
midiendo, no de oído:

- nivel RMS en dBFS, en ventanas fijas
- energía por banda mediante FFT, para comprobar que un pasa-bajos
  efectivamente atenúa donde debe
- distorsión armónica total, para comprobar que la saturación realmente
  distorsiona y cuánto
- desviación de altura en cents, para medir la profundidad del
  wow/flutter
- un detector de clics, calibrado para atrapar discontinuidades en los
  bordes de pinchado que ningún oyente humano está presente para
  escuchar

Un render "golden" fija el sonido exacto de una sesión completa y
guionada de principio a fin. Si cambia, algo en el camino de señal
cambió, y la razón tiene que entenderse y escribirse antes de que el
nuevo render sea bendecido como correcto, nunca al revés.

## Cambiar una decisión establecida

La forma del producto - cuatro pistas, grabación destructiva, la cadena
de DSP específica - está escrita como una especificación formal, y se
trata como una constitución, no como una sugerencia. Cualquier cosa que
revierta una decisión establecida y visible para el usuario requiere
una propuesta escrita y una revisión adversarial antes de que se
escriba una línea de implementación. Tampoco es una formalidad: la
propuesta que reemplazó la mezcla mono original por un bus de mezcla
estéreo en tiempo real pasó por doce rondas de revisión a lo largo de
trece revisiones antes de ser aprobada, y todas menos la última
encontraron un error real o un hueco real, incluidos errores en código
ya publicado que no tenían nada que ver con la propuesta en sí. A
ninguna se le dio el visto bueno de trámite. Mirá [Estado](status.html)
para ver cómo va eso.
