# Changelog del add-on

Cada versión publicada aparece aquí con un resumen de lo que cambia para quien usa FutureFin como
add-on de Home Assistant. El detalle técnico completo — cifras exactas, issues y el porqué de cada
decisión — vive en el
[CHANGELOG del proyecto](https://github.com/maxlainz/FutureFin/blob/main/CHANGELOG.md).

## 4.10.0

- **Vender para vivir ahora paga impuestos, como en la vida real**: la simulación descuenta el
  IRPF del ahorro en cada venta (jubilado o no), así que tu patrimonio proyectado baja — antes
  era estructuralmente optimista. Puedes ajustar qué parte de cada venta es ganancia gravable
  (de serie, todo: lo más prudente; si tu cartera es mitad coste, 0,5 es más realista).
- **Tu objetivo de jubilación se calcula mes a mes sobre lo que de verdad necesitarás**: con
  pensión declarada e inflación, el objetivo de los años lejanos SUBE (antes se quedaba corto).
  El número que ves hoy en el formulario no cambia.
- La línea «aportado» del gráfico ahora BAJA cuando vendes (y el sobrante del jubilado cuenta
  como aportado), y la tarjeta Autonomía descuenta los mismos impuestos que la simulación.
- Aviso honesto: patrimonio más bajo + objetivo más alto pueden retrasar tu fecha de jubilación
  por las dos puntas a la vez. No es un error nuevo — es el optimismo antiguo, retirado.

## 4.9.0

- **La inflación ahora encarece tu gasto mes a mes** (el de hoy y el de jubilación); tus ingresos
  se quedan planos a propósito — las subidas de sueldo hay que peleárselas, la simulación no las
  regala. Las curvas y la fecha de jubilación de cualquier plan con inflación configurada cambian:
  el modelo era estructuralmente optimista y deja de serlo.
- Puedes poner la inflación **en negativo** (hasta −2 %) para estresar tu plan con deflación, y
  las instalaciones nuevas nacen asumiendo un 2,5 % (el objetivo del BCE) en vez de 0 %.
- La proyección llega hasta la **edad que tú elijas** (85–105, de serie 90): subirla revela si tu
  plan aguanta la cola de longevidad. El margen al final se lee en la propia curva.

## 4.8.0

- El objetivo de jubilación ahora cuenta la deuda que te queda: a la cifra de siempre se le suman
  todas las cuotas pendientes de tus préstamos (y lo que quede sin amortizar al final del plan).
  Amortizar deuda **baja tu objetivo**, como debe ser.
- El cruce de jubilación se decide con tu patrimonio **líquido** (lo que puedes vender), no con el
  total: tu vivienda ya no «paga» el objetivo. Y una vez jubilado, jubilado — el simulador ya no
  entra y sale de la jubilación mes a mes.
- La tarjeta «Autonomía» solo dice «indefinida» si tus líquidos tienen rentabilidad esperada
  positiva: el dinero parado al 0 % siempre se agota, y ahora se dice en meses. El cálculo además
  vacía primero las cuentas que menos rinden, igual que la simulación.
- El gasto promedio de Movimientos ya no se diluye con meses sin clasificar, y usa la misma
  ventana de meses que la proyección (anclada a hoy).
- Varias cifras del panel («caja del mes», partidas vencidas) convergen con lo que el motor
  simula de verdad.

## 4.7.0

- Las deudas ahora se simulan como los préstamos reales: el modelo por defecto pasa a ser el
  **sistema francés** (interés + amortización en cada cuota). Si tenías una deuda con tipo de
  interés y cuota mensual guardada con el modelo antiguo «cuota fija», se convierte
  automáticamente y tu proyección empieza a contar los intereses que ese préstamo siempre tuvo:
  verás la deuda durar más y el patrimonio proyectado bajar — es el número honesto, no un error.
- Modelos nuevos de verdad: «solo intereses» cobra exactamente el interés del período (como una
  carencia real), y las revolving usan su cuota mínima real (porcentaje del saldo con suelo en
  euros).
- Un préstamo cuyo plan de pagos terminó con capital pendiente ya no desaparece de tus números:
  sigue visible, marcado «plan vencido con saldo», restando en tu patrimonio.
- El campo del tipo de interés pasa a llamarse **TIN** (lo que siempre fue); las cifras no
  cambian por esto.
- El asistente de IA puede simular amortizaciones anticipadas con su comisión legal (2 % por
  defecto) y con la opción de reducir cuota en vez de acortar plazo.
- El histórico de patrimonio aguanta mejor las fotos incompletas (un elemento que falte en una
  captura ya no hunde la gráfica) y recuerda el modelo de cada deuda al interpolar el pasado.
- Copias de seguridad: formato v11 (las anteriores siguen importando, como siempre).

## 4.6.0

- El asistente de IA (chat MCP) deja de recalcular por su cuenta cifras que el servidor ya sabía:
  ahora también avisa cuándo se agota el patrimonio proyectado, qué parte del gasto queda sin
  cubrir por ningún activo, y si una hipoteca con cuota insuficiente está creciendo en vez de
  amortizarse. Antes esos casos podían pasar desapercibidos.
- La pantalla de Jubilación deja de recalcular «a ojo» el primer cruce con tu objetivo FIRE y lee
  la cifra exacta del servidor — en instalaciones con tramos altos de ahorro podía llegar a
  mostrar un objetivo hasta un 20 % distinto del real.
- Si ya has cruzado tu objetivo, el gráfico de patrimonio muestra el horizonte completo en vez de
  cortarlo a los 12 meses siguientes al cruce.
- Varias fichas de ayuda («i») de la app quedan más completas y precisas.

## 4.5.0

- Guardar `null` en un campo desde la API o desde una herramienta del asistente de IA ahora lo
  borra de verdad — en seis campos concretos, el servidor respondía «hecho» sin cambiar nada.
- El techo del tope de una regla de reparto («hasta X €/mes») se calcula siempre, también en
  meses sin caja sobrante o ya jubilado.
- Corregido un fallo donde un tipo de interés tecleado con coma en vez de punto (por ejemplo
  «3,50» leído como 350) podía colarse sin aviso; el máximo admitido pasa a ser 100 %/año.
- Corregido un fallo en el formulario de escalado del ahorro por tramos: un umbral tecleado como
  «6.000 €» podía interpretarse como 6 € y disparar el objetivo FIRE calculado hasta un 13 % por
  encima del real, en silencio.
- Una rentabilidad esperada absurdamente alta ya no puede tumbar la proyección con un error
  interno; ahora se rechaza con un mensaje claro.
- Varias correcciones de coherencia en el asistente de IA (nombres de parámetro, permisos,
  validación de aportes).

## 4.4.1

- Ronda de correcciones sin cambios de comportamiento visible: mensajes de error más precisos y
  documentación del asistente de IA (MCP) alineada con lo que el servidor hace de verdad.

## 4.4.0

- El asistente de IA (MCP) pasa de 52 a 68 herramientas: ahora puede decirte cuánto pagas de
  intereses en cada deuda y cuándo terminas de pagarla, simular si compensa amortizar antes,
  sumar gastos filtrados sin tener que contarlos a mano, mostrar el patrimonio en euros de hoy, y
  anotar movimientos pasados directamente desde la conversación.
- Varias cifras que el asistente podía enseñar mal quedan corregidas: el patrimonio histórico, el
  objetivo FIRE del mes exacto de jubilación, y los meses sin movimientos (que se leían como gasto
  cero en vez de «sin datos»).
- Si dejas correr el asistente sin supervisión: cada escritura queda auditada (quién, con qué
  credencial, qué cambió), se pueden emitir credenciales de solo lectura, y las acciones
  irreversibles piden confirmación en dos pasos antes de ejecutarse.

## 4.3.1

- Nueva opción `ha_sso_url`: habilita el botón «Entrar con Home Assistant» al
  abrir FutureFin **fuera del panel** (puerto directo o túnel). Vacía = apagado.
- Con ella, autorizar el conector MCP/OAuth ya no exige una segunda cuenta con
  contraseña. FutureFin no guarda ninguna credencial de Home Assistant.

## 4.3.0

- Primera versión de FutureFin como add-on de Home Assistant.
- Panel en la barra lateral vía ingress.
- Inicio de sesión con la identidad de Home Assistant (opción `sso`).
- Puerto directo opcional (deshabilitado por defecto) para MCP y OAuth, que no
  pueden funcionar a través del ingress.
