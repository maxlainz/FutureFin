/**
 * Catálogo de mensajes de error EN ESPAÑOL, indexado por el código estable que devuelve la API.
 *
 * Por qué existe: hasta 3.10.0 el mensaje del backend —inglés y técnico— se pintaba literalmente
 * en la UI. Registrarse con un usuario ya existente mostraba «resource conflict»; un CSV con otra
 * divisa, «currency_mismatch: row 3 has currency 'USD'…». La API sigue
 * hablando inglés (es superficie para desarrolladores, OpenAPI y clientes de terceros); la
 * traducción vive aquí.
 *
 * Contrato: `ErrorBody.code` de la API (ver `apps/api/src/error.rs`). El backend lo saca del
 * prefijo `snake_code:` del mensaje; sin prefijo válido cae a la clase HTTP (`bad_request`,
 * `conflict`…), que también está en este catálogo.
 *
 * Reglas de estilo, iguales que el resto de la UI:
 *  - Frase completa, en español, dirigida a la persona: qué ha pasado y qué puede hacer.
 *  - Sin nombres de campo del API, sin códigos, sin jerga HTTP. El detalle técnico va aparte.
 *  - Sin signos de exclamación ni disculpas.
 */

/** Frases por código estable. Cada entrada es lo ÚNICO que ve el usuario. */
export const ERROR_MESSAGES: Record<string, string> = {
  // ── Clases HTTP: el último recurso cuando el error no trae código propio ──────────
  bad_request:
    "Los datos enviados no son válidos. Revisa el formulario e inténtalo de nuevo.",
  unprocessable:
    "No se puede guardar con esos valores. Revisa los datos e inténtalo de nuevo.",
  unauthorized: "Tu sesión ha caducado. Vuelve a iniciar sesión.",
  forbidden: "No tienes permisos para hacer esto.",
  not_found: "No se ha encontrado lo que buscabas. Puede que se haya borrado.",
  conflict: "Ese valor ya existe. Elige otro.",
  unavailable:
    "El servicio no está disponible ahora mismo. Inténtalo en unos segundos.",
  internal:
    "Algo ha fallado en el servidor. Inténtalo de nuevo en unos segundos.",

  // ── Generados por el propio cliente, no por la API ────────────────────────────────
  // No vienen del fixture del backend: van declarados en `CLIENT_ONLY` dentro de
  // `errorMessages.test.ts`, que si no los denunciaría como entradas huérfanas.
  empty_json_body:
    "La respuesta del servidor no es válida. Comprueba que la API está en marcha.",
  network_error:
    "No se ha podido conectar con el servidor. Comprueba tu conexión y que FutureFin sigue en marcha.",

  // ── Cuenta y acceso ───────────────────────────────────────────────────────────────────
  username_taken: "Ese nombre de usuario ya está registrado. Elige otro.",
  username_length: "El nombre de usuario debe tener entre 3 y 64 caracteres.",
  username_whitespace:
    "El nombre de usuario no puede empezar ni acabar con espacios.",
  username_charset:
    "El nombre de usuario solo admite letras, números y los signos . _ y -",
  password_length: "La contraseña debe tener entre 12 y 256 caracteres.",
  password_hash_failed:
    "No se ha podido guardar la contraseña. Inténtalo de nuevo.",
  birth_date_format: "La fecha de nacimiento debe tener el formato AAAA-MM-DD.",
  birth_date_type: "La fecha de nacimiento no es válida.",
  birth_date_future: "La fecha de nacimiento no puede ser futura.",
  birth_date_too_old: "La fecha de nacimiento debe ser posterior a 1900.",
  installation_race:
    "Otro registro ha creado el hogar justo antes que el tuyo. Vuelve a intentarlo.",
  membership_role_invalid:
    "Ese rol no existe. Elige propietario, miembro o visor.",
  password_unchanged:
    "La contraseña nueva es la misma que la actual. Elige una distinta.",
  current_password_invalid:
    "La contraseña actual no es correcta. Vuelve a escribirla.",
  last_owner:
    "El hogar no puede quedarse sin ningún propietario. Nombra antes a otro propietario.",
  sso_account_no_password:
    "Esta cuenta entra por Home Assistant y no tiene contraseña.",
  sso_disabled:
    "Este servidor no admite la entrada automática desde Home Assistant.",
  sso_untrusted_peer:
    "La entrada automática solo funciona desde Home Assistant. Abre FutureFin desde ahí.",
  sso_bad_identity:
    "Home Assistant no ha enviado una identidad válida. Vuelve a abrir el add-on.",
  sso_username_unavailable:
    "No se ha podido crear tu cuenta porque ese nombre de usuario ya está cogido. Avisa al propietario del hogar.",
  // «Entrar con Home Assistant»: estos códigos NO llegan en el cuerpo de una respuesta, sino
  // como `?ha_error=` en la vuelta del redirect. Se traducen aquí igual que el resto para que
  // haya un único sitio donde vive la frase que ve el usuario.
  ha_sso_disabled:
    "Esta instalación no tiene configurado el acceso con Home Assistant. Pídele al propietario que rellene «URL de Home Assistant» en las opciones del add-on.",
  ha_state_mismatch:
    "La vuelta desde Home Assistant no se ha podido verificar. Vuelve a pulsar «Entrar con Home Assistant».",
  ha_exchange_failed:
    "Home Assistant no ha confirmado el acceso. Comprueba que sigue en marcha y vuelve a intentarlo.",
  ha_identity_failed:
    "No se ha podido leer tu usuario de Home Assistant. Vuelve a intentarlo en unos segundos.",

  // ── Copia de seguridad (.ffbackup) ────────────────────────────────────────────────────
  backup_wrong_password:
    "La contraseña del archivo no es correcta. Es la que elegiste al exportarlo, no la de tu cuenta.",
  backup_not_a_ffbackup_file:
    "Ese archivo no es una copia de seguridad de FutureFin. Elige un archivo .ffbackup.",
  backup_file_corrupt:
    "El archivo está dañado o incompleto. Vuelve a exportar la copia y prueba otra vez.",
  backup_format_version_unsupported:
    "La copia se creó con una versión más reciente de FutureFin. Actualiza el servidor para poder importarla.",
  backup_schema_version_unsupported:
    "La copia es de una versión posterior a la de este servidor. Actualiza FutureFin para poder importarla.",
  backup_schema_version_unknown:
    "La copia declara un formato que FutureFin no reconoce. Usa otra copia.",
  backup_crypto_params_unsupported:
    "La copia está cifrada con un método que este servidor no reconoce. Comprueba que la generó FutureFin.",
  backup_file_too_large:
    "El contenido de la copia es demasiado grande para importarlo. Si la generó FutureFin, avísanos.",
  backup_payload_malformed:
    "El contenido de la copia no tiene la forma esperada. El archivo está dañado; usa otra copia.",
  backup_category_reference_missing:
    "La copia usa una categoría que ella misma no incluye, así que está incompleta. Vuelve a exportarla.",
  backup_reference_out_of_range:
    "La copia tiene referencias internas rotas. Está dañada; usa otra copia.",
  backup_reference_not_distinct:
    "La copia contiene un par de transferencias que se apunta a sí mismo. Está dañada; usa otra copia.",
  confirm_required:
    "Falta confirmar la operación. Marca la confirmación para continuar.",
  file_b64_invalid:
    "El archivo no se ha podido leer al subirlo. Vuelve a seleccionarlo e inténtalo otra vez.",
  original_filename_too_long:
    "El nombre del archivo es demasiado largo. Renómbralo antes de subirlo.",

  // ── Importar movimientos (CSV) ────────────────────────────────────────────────────────
  csv_preset_unrecognized:
    "No se reconoce el formato de ese CSV. Ahora mismo se admiten los extractos de MyInvestor y N26.",
  csv_parse_error:
    "El CSV no se ha podido leer. Compruébalo y vuelve a exportarlo de tu banco.",
  csv_date_invalid:
    "Hay una fecha que no se entiende en el CSV. Exporta el extracto otra vez sin modificarlo.",
  csv_amount_ambiguous:
    "Un importe del archivo usa el punto como separador decimal y este formato espera coma. Comprueba que el archivo es del banco que has elegido.",
  csv_amount_invalid:
    "Hay un importe que no se entiende en el CSV. Exporta el extracto otra vez sin modificarlo.",
  currency_mismatch:
    "El archivo trae movimientos en otra divisa. Solo se pueden importar los que estén en la divisa de tu hogar.",
  preview_confirm_mismatch:
    "El archivo ha cambiado desde la vista previa. Vuelve a empezar la importación.",
  pending_assignments_too_many:
    "Demasiadas asignaciones pendientes en esta sesión. Confirma el archivo y sigue con el resto.",
  transaction_currency_invalid: "Esa divisa no es válida.",
  transaction_kind_invalid:
    "El tipo de movimiento no es válido. Elige gasto, ingreso o inversión.",
  transaction_concept_empty: "El concepto no puede quedar vacío.",
  transaction_concept_too_long: "El concepto es demasiado largo. Acórtalo.",
  transaction_notes_too_long: "Las notas son demasiado largas. Recórtalas.",
  transaction_category_requires_kind:
    "Para poner categoría hay que indicar también si es gasto, ingreso o inversión.",
  transactions_not_found:
    "Alguno de los movimientos ya no existe, así que no se ha cambiado nada. Recarga la lista e inténtalo otra vez.",
  amount_zero: "El importe no puede ser cero.",
  mcp_disabled:
    "El servidor MCP está desactivado en esta instalación.",
  confirm_token_required:
    "Esa acción no se puede confirmar a ciegas: hay que previsualizarla antes y confirmar con el código que devuelve el preview.",
  confirm_token_invalid:
    "El código de confirmación no vale: es de otra operación, ya se usó o ha caducado (dura 10 minutos y sirve una sola vez). Vuelve a previsualizar.",
  confirm_token_stale:
    "Los datos han cambiado desde la previsualización, así que la confirmación ya no describe lo que iba a pasar. Vuelve a previsualizar y revisa las cifras nuevas.",
  idempotency_key_invalid:
    "La clave de idempotencia debe tener entre 1 y 200 caracteres.",
  idempotency_key_conflict:
    "Esa clave de idempotencia ya creó un movimiento distinto. Gana el primero: usa una clave nueva para un movimiento nuevo.",
  idempotency_key_in_flight:
    "Otra petición está usando esa clave de idempotencia ahora mismo. Inténtalo de nuevo.",
  idempotency_key_batch_unsupported:
    "La clave de idempotencia solo vale para dar de alta un movimiento suelto, no para un lote.",
  budget_entry_is_liability_derived:
    "Esa partida es la cuota de un pasivo: se edita o se elimina desde el pasivo, no desde el presupuesto.",
  // --- Fase 6: capacidades nuevas (agregación, calendario de deuda, deflactado, cascada) ---
  category_filter_exclusive:
    "No puedes filtrar por categoría y por «sin categoría» a la vez. Elige una de las dos cosas.",
  transfer_match_not_found:
    "Esa sugerencia de transferencia ya no existe: vuelve a pedir la lista y confirma una de las actuales.",
  window_days_out_of_range:
    "La ventana debe estar entre 1 y 365 días.",
  sink_creation_not_allowed:
    "No se puede crear desde aquí la regla de resto sin tope (el sumidero de la cascada). Ponle un tope, o cambia el sumidero desde la app.",
  deflate_date_in_past:
    "Solo se pueden pasar a euros de hoy importes de una fecha futura.",
  deflate_month_out_of_range:
    "Ese mes queda fuera del horizonte de la proyección.",
  deflate_timing_ambiguous:
    "Indica la fecha o el número de mes, pero no los dos.",
  schedule_from_month_out_of_range:
    "El mes de inicio del calendario queda fuera del plan de pago.",
  schedule_window_out_of_range:
    "La ventana del calendario de amortización está fuera del rango permitido.",
  liability_not_in_scope:
    "Ese pasivo no es de este hogar, o ya no existe.",
  liability_override_duplicate:
    "Has repetido el mismo pasivo dos veces en la simulación. Deja una sola entrada por deuda.",
  liability_override_empty:
    "Esa simulación de deuda no cambia nada: indica amortización extra, una entrega puntual o un TIN.",
  liability_override_needs_payment_plan:
    "Para simular esa deuda hace falta que tenga cuota y periodicidad.",
  early_repayment_fee_out_of_range:
    "La compensación por amortizar antes debe estar entre 0 y 2 % (el tope legal a tipo fijo).",
  early_repayment_effect_invalid:
    "Ese efecto de amortización no existe. Elige acortar plazo o reducir cuota.",
  liability_early_repayment_axis_needs_amortization:
    "La comisión y el efecto de amortizar solo aplican si el escenario amortiza algo. Añade una amortización mensual o puntual.",
  liability_override_revolving_needs_minimums:
    "Para simular esa deuda como revolving necesita su cuota mínima (porcentaje del saldo o suelo en euros) guardada en la ficha.",
  reduce_payment_ignored_by_repayment_model:
    "Reducir cuota no hace nada en una revolving: su cuota es el mínimo del saldo, no la declarada. Usa acortar plazo.",
  liability_overrides_unavailable_in_real_expense_mode:
    "Con el gasto tomado de tus movimientos reales, la cuota ya está dentro del gasto: simular la deuda aparte la contaría dos veces.",
  liability_repayment_model_needs_apr:
    "Ese modelo de amortización necesita un TIN mayor que cero.",
  liability_apr_negative:
    "El TIN no puede ser negativo.",
  liability_apr_ignored_by_repayment_model:
    "Ese modelo de amortización no usa el TIN: la cuota va íntegra a principal.",
  liability_extra_principal_negative:
    "La amortización extra no puede ser negativa.",
  liability_lump_sum_amount_required:
    "Indica el importe de la entrega puntual.",
  liability_lump_sum_not_positive:
    "La entrega puntual tiene que ser mayor que cero.",
  liability_lump_sum_timing_ambiguous:
    "Indica la fecha o el número de mes de la entrega, pero no los dos.",
  liability_lump_sum_month_out_of_range:
    "El mes de la entrega puntual queda fuera del horizonte de la proyección.",
  liability_lump_sum_date_out_of_horizon:
    "La fecha de la entrega puntual queda fuera del horizonte de la proyección.",
  uuid_invalid:
    "Ese identificador no tiene el formato esperado. Cópialo tal cual del listado en vez de escribirlo a mano.",
  date_invalid:
    "Esa fecha no se entiende. Usa el formato AAAA-MM-DD (por ejemplo, 2026-03-01).",
  due_date_set_and_clear:
    "No se puede poner y borrar la fecha prevista a la vez. Elige una de las dos cosas.",
  purchase_price_set_and_clear:
    "No se puede poner y borrar el precio de compra a la vez. Elige una de las dos cosas.",
  expense_end_set_and_clear:
    "No se puede poner y borrar la fecha de fin del gasto a la vez. Elige una de las dos cosas.",
  repayment_model_state_invalid:
    "A esa deuda le faltan varios datos para el modelo elegido: revisa cuota, periodicidad y TIN.",
  window_months_out_of_range:
    "La ventana pedida está fuera del rango permitido. Ajusta el número de meses.",
  due_date_out_of_range:
    "Esa fecha está demasiado lejos: como mucho, 100 años vista.",
  payment_end_date_out_of_range:
    "La fecha de fin del plan está demasiado lejos: como mucho, 100 años vista.",
  amount_out_of_range:
    "El importe es demasiado grande: caben hasta 14 dígitos enteros y 4 decimales.",
  amount_sign_mismatch:
    "El signo del importe no cuadra con el tipo: los ingresos van en positivo, y los gastos y los traspasos a inversión en negativo.",
  op_date_in_future:
    "No puedes apuntar un movimiento con fecha futura. Si es algo que aún no ha pasado, va en «Próximos».",
  batch_empty: "No has seleccionado ningún movimiento. Elige al menos uno.",
  batch_too_large:
    "Has seleccionado demasiados movimientos de una vez. Divídelos en tandas más pequeñas.",
  nothing_to_update:
    "No has indicado ningún cambio. Elige un tipo, una categoría o unas notas antes de guardar.",
  category_set_and_clear:
    "Has pedido asignar y borrar la categoría a la vez. Elige una de las dos.",
  notes_set_and_clear:
    "Has pedido escribir y borrar las notas a la vez. Elige una de las dos.",
  value_date_set_and_clear:
    "No se puede poner y borrar la fecha valor a la vez. Elige una de las dos cosas.",
  linked_asset_set_and_clear:
    "No se puede enlazar y desenlazar el activo a la vez. Elige una de las dos cosas.",
  linked_liability_set_and_clear:
    "No se puede enlazar y desenlazar el pasivo a la vez. Elige una de las dos cosas.",
  concept_contains_length:
    "El texto de búsqueda debe tener entre 1 y 200 caracteres.",
  month_and_range_exclusive:
    "Has pedido a la vez un mes y un rango de fechas. Usa uno de los dos.",
  date_range_inverted:
    "La fecha inicial es posterior a la final. Intercámbialas.",
  amount_range_inverted:
    "El importe mínimo es mayor que el máximo. Recuerda que los gastos son negativos.",
  month_format_invalid: "El mes no es válido. Usa el formato AAAA-MM.",
  month_out_of_range: "El mes debe estar entre 1 y 12.",
  year_out_of_range: "El año debe estar entre 1900 y 3000.",
  year_month_incomplete: "Falta el año o el mes. Indica los dos, o ninguno.",

  // ── Transferencias y conciliación ─────────────────────────────────────────────────────
  already_reconciled:
    "Ese movimiento ya está conciliado con otro. Desconcílialo primero.",
  not_reconciled:
    "Ese movimiento no está conciliado, así que no hay nada que deshacer.",
  transfer_same_transaction:
    "No puedes conciliar un movimiento consigo mismo. Elige su contrapartida.",
  transfer_currency_mismatch:
    "Los dos movimientos están en divisas distintas y no pueden formar una transferencia.",
  transfer_amounts_not_opposite:
    "Los importes no son exactamente opuestos, así que no parecen las dos patas de una transferencia.",

  // ── Reglas de categorización y recurrentes ────────────────────────────────────────────
  rule_match_kind_invalid: "El tipo de coincidencia de la regla no es válido.",
  rule_pattern_empty: "El patrón de la regla no puede quedar vacío.",
  rule_pattern_too_long: "El patrón de la regla es demasiado largo. Acórtalo.",
  rule_assign_kind_invalid:
    "El tipo que asigna la regla no es válido. Elige gasto, ingreso o inversión.",
  rule_assign_kind_required:
    "La regla necesita saber qué asignar. Elige gasto, ingreso o inversión.",
  rule_assign_category_requires_kind:
    "Para que la regla asigne una categoría hay que indicar también el tipo.",
  rule_patch_empty:
    "No has indicado ningún cambio en la regla. Modifica algo antes de guardar.",
  rule_patch_conflict:
    "Estás poniendo y quitando el mismo dato de la regla a la vez. Elige una de las dos cosas.",
  rule_duplicate:
    "Ya tienes una regla con ese texto para ese banco. Edítala en vez de crear otra.",
  rule_not_applicable:
    "Esa regla no asigna ningún tipo, así que no se puede aplicar a los movimientos que ya tienes. Edítala primero.",
  apply_to_existing_invalid:
    "Elige a qué aplicar la regla: a nada, solo a lo que no tenga categoría, o a todo.",
  recurring_rule_kind_invalid:
    "El tipo del movimiento recurrente no es válido. Elige gasto, ingreso o inversión.",
  recurring_rule_concept_empty:
    "El concepto del movimiento recurrente no puede quedar vacío.",
  recurring_rule_concept_too_long:
    "El concepto del movimiento recurrente es demasiado largo. Acórtalo.",
  recurring_rule_amount_zero:
    "El importe del movimiento recurrente no puede ser cero.",
  recurrence_too_old:
    "La fecha de inicio del recurrente es demasiado antigua. Acércala.",
  savings_no_category:
    "Los movimientos de inversión no llevan categoría. Quítala para poder guardar.",

  // ── Activos, pasivos y categorías ─────────────────────────────────────────────────────
  name_empty: "Escribe un nombre para poder guardar.",
  name_too_long: "El nombre es demasiado largo. Como máximo 200 caracteres.",
  label_empty: "Escribe un nombre para poder guardar.",
  label_too_long: "El nombre es demasiado largo. Como máximo 200 caracteres.",
  title_empty: "Escribe un título para poder guardar.",
  title_too_long: "El título es demasiado largo. Como máximo 200 caracteres.",
  notes_too_long:
    "Las notas son demasiado largas. Como máximo 4.000 caracteres.",
  type_tag_too_long:
    "La etiqueta de tipo es demasiado larga. Como máximo 120 caracteres.",
  amount_negative:
    "Esa cantidad no puede ser negativa. Escribe cero o un número positivo.",
  negative_amount:
    "Esa cantidad no puede ser negativa. Escribe cero o un número positivo.",
  amount_not_positive: "El importe debe ser mayor que cero.",
  decimal_invalid:
    "Esa cantidad no se entiende como número. Escríbela solo con cifras y, si hace falta, un decimal.",
  return_percent_too_low:
    "La rentabilidad anual no puede ser del -100 % o menos.",
  volatility_out_of_range:
    "La volatilidad anual debe estar entre 0 y 100 %. Como referencia: renta variable global ~15-18 %, renta fija ~4-6 %, efectivo 0.",
  patch_empty: "No has cambiado nada. Modifica algún dato antes de guardar.",
  category_wrong_scope:
    "Esa categoría no sirve para este apartado. Elige una del tipo correcto.",
  category_not_found:
    "Esa categoría no existe en tu hogar. Elige una de la lista.",
  category_scope_invalid:
    "El tipo de categoría no es válido. Usa activo, pasivo, ingreso o gasto.",
  category_scope_mismatch:
    "La categoría es de otro tipo del que hace falta aquí.",
  category_not_in_installation:
    "Esa categoría no existe en esta instalación. Elige una de la lista.",
  category_in_use:
    "Esa categoría está en uso. Elige a qué categoría traspasar sus datos antes de borrarla.",
  category_required:
    "Los ingresos y los gastos necesitan categoría. Elige una en cada fila antes de continuar.",
  category_is_fallback:
    "Esa es la categoría por defecto de su ámbito y no se puede borrar. Marca antes otra como predeterminada.",
  fallback_cannot_be_unset:
    "La categoría por defecto no se puede quitar, solo trasladar: marca otra como predeterminada y esta dejará de serlo.",
  fallback_scope_invalid:
    "Solo las categorías de ingreso y de gasto pueden ser la predeterminada.",
  fallback_category_missing:
    "Tu hogar no tiene categoría por defecto para ese ámbito. Crea una en Ajustes → Categorías y márcala como predeterminada.",
  remap_to_same_category:
    "No puedes traspasar los datos a la misma categoría que estás borrando.",
  remap_to_not_found:
    "La categoría de destino no existe. Elige una de la lista.",
  remap_to_scope_mismatch:
    "La categoría de destino es de otro tipo. Elige una del mismo tipo.",
  invalid_kind: "Ese tipo no es válido.",
  kinds_empty: "Selecciona al menos un tipo.",
  expense_category_wrong_scope:
    "Para la cuota hace falta una categoría de gasto de tu hogar.",
  principal_required:
    "Falta el capital pendiente. Escríbelo, o activa el cálculo a partir del plan de pagos.",
  payment_frequency_invalid:
    "La periodicidad del pago solo puede ser mensual o semanal.",
  payment_amount_not_positive: "La cuota debe ser mayor que cero.",
  payment_pair_incomplete:
    "Falta la mitad del plan de pagos. Indica cuota y periodicidad, o quita las dos.",
  payment_end_date_in_past:
    "La fecha de fin de pagos ya ha pasado. Pon una de hoy en adelante.",
  payment_schedule_too_long:
    "El plan de pagos abarca demasiado tiempo. Acerca la fecha de fin.",
  payment_schedule_overflow:
    "La fecha de fin de pagos queda fuera del calendario que se puede calcular. Pon una más cercana.",
  payment_schedule_empty:
    "Con esa cuota y esa fecha de fin no sale ni un solo recibo. Revísalas.",
  payment_amount_required_for_derived_principal:
    "Para calcular el capital pendiente a partir del plan hace falta la cuota.",
  payment_frequency_required_for_derived_principal:
    "Para calcular el capital pendiente a partir del plan hace falta la periodicidad.",
  payment_end_date_required_for_derived_principal:
    "Para calcular el capital pendiente a partir del plan hace falta la fecha del último pago.",
  repayment_model_invalid:
    "Ese modelo de deuda no existe. Elige francés, sin intereses, solo intereses o revolving.",
  apr_required_for_model:
    "Ese modelo de deuda devenga interés: hace falta el TIN (mayor que cero).",
  apr_forbidden_for_model:
    "Una deuda sin intereses no lleva TIN. Quítalo o elige francés, solo intereses o revolving.",
  payment_plan_required_for_model:
    "Ese modelo de deuda necesita plan de pagos: indica cuota y periodicidad.",
  derive_not_supported_for_model:
    "El capital pendiente solo se puede calcular del plan en las deudas sin intereses o francesas.",
  weekly_not_supported_for_model:
    "La periodicidad semanal solo vale para deudas sin intereses.",
  revolving_minimum_required:
    "Una revolving necesita su cuota mínima: porcentaje del saldo o suelo en euros, mayor que cero.",
  revolving_minimum_forbidden_for_model:
    "La cuota mínima (porcentaje del saldo y suelo) es solo de las revolving. Quítala en los demás modelos.",
  apr_percent_set_and_clear:
    "No se puede fijar el TIN y borrarlo a la vez. Manda solo uno de los dos.",

  // ── Presupuesto y próximos ────────────────────────────────────────────────────────────
  budget_entry_category_scope_unsupported:
    "Esta línea está enganchada a una categoría que ya no es de ingreso ni de gasto. Asígnale una válida.",
  planning_flow_category_scope_unsupported:
    "Este movimiento previsto está enganchado a una categoría que ya no es de ingreso ni de gasto. Asígnale una válida.",
  end_condition_conflict:
    "No puedes marcar que acaba con la jubilación y ponerle además una fecha de fin. Deja solo una.",
  due_date_type:
    "La fecha prevista no es válida. Escribe una fecha o déjala vacía.",
  due_date_format: "La fecha prevista debe tener el formato AAAA-MM-DD.",
  date_required: "Falta la fecha, o no tiene un formato válido.",
  // #148 — flujos recurrentes con ventana
  amount_basis_invalid:
    "El tipo del flujo debe ser puntual (one_off) o recurrente (per_month).",
  window_requires_per_month:
    "El periodo (desde/hasta) solo aplica a un flujo recurrente. Cámbialo a recurrente o quita las fechas.",
  per_month_excludes_due_date:
    "Un flujo recurrente usa un periodo, no una fecha de vencimiento. Quita la fecha prevista.",
  window_start_required:
    "Un flujo recurrente necesita la fecha de inicio de su periodo.",
  window_end_before_start:
    "El fin del periodo no puede ser anterior a su inicio.",
  window_date_type:
    "Las fechas del periodo no son válidas. Escribe una fecha o déjala vacía.",
  window_date_format: "Las fechas del periodo deben tener el formato AAAA-MM-DD.",
  window_date_out_of_range:
    "Las fechas del periodo no pueden estar a más de 100 años vista.",
  window_start_set_and_clear:
    "No puedes fijar y borrar el inicio del periodo a la vez.",
  window_end_set_and_clear:
    "No puedes fijar y borrar el fin del periodo a la vez.",
  backup_amount_basis_invalid:
    "El backup trae un movimiento previsto con un tipo desconocido. El archivo puede estar dañado.",

  // ── Reparto del sobrante ──────────────────────────────────────────────────────────────
  rule_kind_invalid:
    "El tipo de regla no es válido. Usa importe fijo, porcentaje o sobrante.",
  amount_required_for_kind:
    "Esta regla necesita una cantidad. Indica el importe o el porcentaje.",
  percent_out_of_range: "El porcentaje debe estar entre 0 y 100.",
  cap_value_negative:
    "El tope no puede ser negativo. Escribe cero o una cifra positiva.",
  cap_kind_invalid:
    "El tipo de tope no es válido. Usa un importe, meses de gasto o un múltiplo del ingreso.",
  cap_pair_incomplete:
    "El tope está a medias. Indica el tipo y su valor, o quita los dos.",
  cap_set_and_clear:
    "Estás poniendo y quitando el tope a la vez. Elige una de las dos cosas.",
  cap_type_invalid:
    "El tope no tiene el formato esperado. Vuelve a configurarlo desde el formulario.",
  target_asset_not_found:
    "El activo de destino no existe o no se ve en esta vista. Elige otro.",
  remainder_required:
    "Esa regla (o ese activo) es tu único sumidero del sobrante: mientras tengas activos no se puede borrar, desactivar ni ponerle tope. Muévelo antes a otro activo.",
  uncapped_remainder_exists:
    "Ya hay una regla de sobrante sin tope: solo puede haber una.",
  sink_must_be_last: "La regla que recoge el sobrante tiene que ir la última.",
  ids_not_unique:
    "El nuevo orden repite alguna regla. Recarga la página y reordénalas otra vez.",
  ids_do_not_match_scope:
    "El nuevo orden no coincide con las reglas que hay ahora. Recarga la página e inténtalo otra vez.",

  // ── Plan de jubilación por usuario (5.0.0) ────────────────────────────────────────────
  strategy: "Esa estrategia de jubilación no existe.",
  target_retirement_age_required:
    "Esa estrategia necesita que digas a qué edad quieres jubilarte.",
  retirement_age_out_of_range:
    "La edad de jubilación tiene que estar entre los 18 años y tu edad límite del horizonte.",
  pension_required_for_bridge:
    "«Puente hasta la pensión» necesita que declares tu pensión: importe y edad a la que empieza.",
  pension_age_out_of_range:
    "La edad a la que empieza la pensión tiene que estar entre los 50 años y tu edad límite del horizonte.",
  pension_amount_not_positive: "El importe de la pensión debe ser mayor que cero.",
  pension_fraction_out_of_range:
    "La parte de la pensión que se cobra en media jornada debe estar entre 0 y 1.",
  partial_age_out_of_range:
    "La edad a la que empieza la media jornada tiene que estar entre los 18 años y tu edad límite del horizonte.",
  partial_not_before_retirement:
    "La media jornada tiene que empezar antes de la jubilación total.",
  partial_income_negative: "El ingreso de la media jornada no puede ser negativo.",
  target_basis: "Esa base del objetivo de jubilación no es válida.",
  bridge_discount_basis: "Esa forma de descontar el puente hasta la pensión no es válida.",
  withdrawal_rule_kind: "Esa regla de retirada no existe.",
  spend_mode: "Esa forma de relacionar la regla de retirada con tu gasto no es válida.",
  expense_basis: "Esa base de gasto para la media jornada no es válida.",
  withdrawal_pct_required: "A esa regla de retirada le falta alguno de sus porcentajes.",
  withdrawal_pct_out_of_range:
    "El porcentaje de la regla de retirada debe ser mayor que 0 y como mucho 20 %.",
  withdrawal_band_out_of_range:
    "La banda y el ajuste de la regla de bandas deben ser mayores que 0 y como mucho 50 %.",
  hybrid_end_pct_not_below_start:
    "En la regla híbrida, el porcentaje final tiene que ser menor que el inicial.",
  cash_buffer_out_of_range: "El colchón de caja debe estar entre 0 y 60 meses.",
  success_threshold_out_of_range:
    "El umbral de éxito debe estar entre el 50 % y el 99 %.",
  field_set_and_clear:
    "Has pedido a la vez fijar y borrar el mismo campo. Elige una de las dos cosas.",

  // ── Filas del hogar: quién puede editarlas ────────────────────────────────────────────
  not_row_owner:
    "Esto es de otra persona del hogar. Puedes verlo, pero solo quien lo creó puede cambiarlo.",
  household_read_only:
    "La vista del hogar es solo de lectura para esta acción. Cambia a «Yo» y vuelve a intentarlo.",

  // ── Ajustes del hogar y del plan ──────────────────────────────────────────────────────
  swr_out_of_range: "La tasa de retirada segura debe estar entre 0 y 4 %.",
  fire_number_mode: "Esa forma de fijar el número FIRE no es válida.",
  fire_manual_amount_required:
    "Has elegido fijar el número FIRE a mano, pero falta la cifra.",
  fire_manual_amount_not_positive:
    "El número FIRE fijado a mano debe ser mayor que cero.",
  savings_source: "Ese origen del ahorro no es válido.",
  avg_window_invalid:
    "Esa ventana del promedio no es válida. Elige 3, 6 o 12 meses, el año en curso o todo el histórico.",
  avg_window_out_of_range:
    "La ventana del promedio está fuera de rango. Elige un número de meses admitido.",
  income_avg_window_mode:
    "Esa forma de contar los meses del promedio de ingresos no es válida. Elige entre los últimos meses con datos o los últimos meses del calendario.",
  expense_avg_window_mode:
    "Esa forma de contar los meses del promedio de gastos no es válida. Elige entre los últimos meses con datos o los últimos meses del calendario.",
  avg_months_out_of_range:
    "El número de meses del promedio está fuera del rango permitido.",
  tax_brackets_empty:
    "Has activado los impuestos pero no hay ningún tramo. Añade al menos uno.",
  tax_bracket_pct_out_of_range:
    "El tipo de un tramo debe estar entre 0 y 99 %.",
  tax_bracket_threshold_not_positive:
    "El límite de un tramo debe ser mayor que cero.",
  tax_brackets_last_must_be_open:
    "El último tramo debe quedar abierto. Quítale el límite superior.",
  tax_brackets_open_not_last:
    "Solo el último tramo puede quedar abierto. Pon un límite a los demás.",
  tax_brackets_ordering_invalid:
    "Los tramos están mal ordenados. Revísalos de menor a mayor.",
  tax_brackets_not_increasing:
    "Los límites de los tramos deben ir siempre a más. Corrige el que se repite o baja.",
  inflation_out_of_range: "La inflación anual debe estar entre -2 y 50 %.",
  horizon_lifespan_age_out_of_range:
    "La edad límite del horizonte debe estar entre 85 y 105 años.",
  taxable_gain_ratio_out_of_range:
    "La fracción de plusvalía gravable debe estar entre 0 y 1 (por ejemplo, 0,5).",
  apr_out_of_range:
    "El TIN anual debe estar entre 0 y 100 %. Si has escrito 350 queriendo decir 3,50, usa la coma decimal.",
  backup_asset_return_invalid:
    "El backup trae un activo con una rentabilidad esperada imposible (≤ −100 %); corrígela en la instalación de origen y vuelve a exportar.",
  backup_asset_volatility_invalid:
    "El backup trae un activo con una volatilidad fuera de rango (debe estar entre 0 y 100 %); corrígela en la instalación de origen y vuelve a exportar.",
  inflation_not_a_number:
    "La inflación no se entiende como número. Escríbela solo con cifras y, si hace falta, un decimal.",
  currency_format_invalid: "El código de moneda debe tener tres letras.",
  currency_unsupported: "Esa moneda todavía no está soportada.",
  show_age_mode_invalid:
    "Esa forma de mostrar el tiempo no es válida. Elige fechas o edades.",
  timezone_invalid:
    "Esa zona horaria no es válida. Usa un nombre como Europe/Madrid.",
  installation_timezone_broken:
    "La zona horaria guardada de tu hogar no es válida. Corrígela en Ajustes para que las fechas vuelvan a cuadrar.",

  // ── Histórico y snapshots ─────────────────────────────────────────────────────────────
  snapshot_kind_invalid: "El tipo de elemento del snapshot no es válido.",
  snapshot_source_invalid: "El origen del snapshot no es válido.",
  snapshot_label_empty: "Cada elemento del snapshot necesita un nombre.",
  snapshot_label_too_long:
    "El nombre de un elemento del snapshot es demasiado largo.",
  snapshot_value_negative:
    "El valor de un elemento del snapshot no puede ser negativo.",
  snapshot_apr_percent_negative:
    "El TIN de un elemento del snapshot no puede ser negativo.",
  snapshot_payment_amount_negative:
    "La cuota de un elemento del snapshot no puede ser negativa.",
  snapshot_payment_frequency_invalid:
    "La periodicidad de un elemento del snapshot solo puede ser mensual o semanal.",
  snapshot_terms_only_for_liabilities:
    "Las condiciones del préstamo solo tienen sentido en un pasivo.",
  snapshot_repayment_model_invalid:
    "Ese modelo de deuda no existe en el snapshot. Elige francés, sin intereses, solo intereses o revolving.",
  snapshot_duplicate_item:
    "El snapshot repite un elemento. Deja uno solo de cada.",
  snapshot_date_in_future: "Un snapshot no puede tener fecha futura.",
  snapshot_date_too_old: "Esa fecha es demasiado antigua para un snapshot.",
  duplicate_item_id: "Hay elementos repetidos. Deja uno solo de cada.",
  terms_only_for_liabilities:
    "Las condiciones del préstamo solo tienen sentido en un pasivo.",
  too_many_items: "El snapshot tiene demasiados elementos. Reduce la lista.",
  daily_window_too_large:
    "El tramo pedido es demasiado largo para el detalle diario. Acórtalo.",
  history_timeline_invalid:
    "No se ha podido reconstruir tu histórico con los datos guardados. Revisa tus snapshots.",
  linked_asset_not_found:
    "El activo vinculado ya no existe. Elige otro o quita el vínculo.",
  linked_liability_not_found:
    "El pasivo vinculado ya no existe. Elige otro o quita el vínculo.",
  category_series_kind_invalid: "Esa serie solo existe para gastos o ingresos.",

  // ── Simulación y proyección ───────────────────────────────────────────────────────────
  engine_rejected_input:
    "Con los datos actuales no se puede calcular la proyección. Revisa activos, reglas de reparto y presupuesto.",
  task_panic:
    "El cálculo de la proyección se ha interrumpido. Vuelve a intentarlo en unos segundos.",
  months_out_of_range:
    "El horizonte de la proyección debe estar entre 12 y 840 meses.",
  retirement_expense_not_positive:
    "El gasto anual en jubilación debe ser mayor que cero.",
  one_off_amount_not_positive:
    "El importe del gasto puntual debe ser mayor que cero.",
  one_off_amount_required:
    "Has indicado cuándo ocurre el gasto puntual, pero no cuánto.",
  one_off_timing_ambiguous:
    "Indica cuándo ocurre el gasto puntual de una sola forma: el mes o la fecha, no las dos.",
  one_off_month_out_of_range:
    "Ese mes queda fuera del horizonte simulado. Elige uno dentro.",
  one_off_date_out_of_horizon:
    "Esa fecha queda fuera del horizonte simulado. Elige otra más cercana.",
  asset_not_in_scope:
    "Uno de los activos de la simulación no existe o no se ve en esta vista. Recarga la página.",

  // ── Parámetros de consulta ────────────────────────────────────────────────────────────
  // La app nunca los provoca: manda siempre valores válidos. Existen porque desde 4.0.0 la API
  // rechaza el valor desconocido en vez de caer al default en silencio, y alguien que llame a la
  // API a mano (o un agente por MCP) sí puede verlos.
  invalid_view: "Esa vista no existe. Elige «solo lo mío» o «todo el hogar».",
  invalid_resolution:
    "Esa resolución no es válida. El detalle del gráfico puede ser semanal o diario.",
  invalid_density: "Esa densidad de la serie no es válida. Puede ser mensual o híbrida.",
  limit_out_of_range:
    "El número de resultados por página está fuera del rango permitido.",

  // ── Tokens, conexiones y sistema ──────────────────────────────────────────────────────
  token_label_length: "El nombre del token debe tener entre 1 y 64 caracteres.",
  token_expiry_out_of_range:
    "La caducidad del token debe estar entre 1 y 3.650 días.",
  token_limit_reached:
    "Has llegado al máximo de tokens. Revoca alguno antes de crear otro.",
  token_scope_invalid:
    "Esos permisos no son válidos. Elige solo lectura, o lectura y escritura.",
  mcp_token_read_only:
    "Ese token es de solo lectura. Crea uno con permiso de escritura en Ajustes → Integraciones si necesitas que Claude modifique datos.",
  mcp_write_disabled:
    "La escritura desde Claude está desactivada en este hogar. El propietario puede activarla en Ajustes → Integraciones.",
  public_url_underivable:
    "No se ha podido determinar la dirección pública de la aplicación. Revisa la configuración del servidor.",
  redirect_uri_invalid:
    "La aplicación que quiere conectarse ha enviado una dirección de retorno inválida. Vuelve a iniciar la conexión desde ella.",
  authorize_error:
    "La autorización no se ha podido completar. Vuelve a iniciarla desde la aplicación.",
  readiness_probe_unexpected:
    "El servidor ha respondido algo inesperado al comprobar su estado. Inténtalo de nuevo en unos segundos.",
};

/** Frase por código de estado, para respuestas sin cuerpo JSON (proxy caído, 413, 502…). */
const STATUS_MESSAGES: Record<number, string> = {
  401: ERROR_MESSAGES.unauthorized,
  403: ERROR_MESSAGES.forbidden,
  404: ERROR_MESSAGES.not_found,
  409: ERROR_MESSAGES.conflict,
  413: "El archivo es demasiado grande. Prueba con uno más pequeño.",
  429: "Demasiadas peticiones seguidas. Espera unos segundos e inténtalo otra vez.",
  500: ERROR_MESSAGES.internal,
  502: "No se ha podido contactar con el servidor. Comprueba que sigue en marcha.",
  503: ERROR_MESSAGES.unavailable,
  504: "El servidor ha tardado demasiado en responder. Inténtalo de nuevo.",
};

const FALLBACK = "Algo ha fallado. Inténtalo de nuevo en unos segundos.";

/**
 * Un 401 al ENVIAR el formulario de acceso no es una sesión caducada: es que el usuario o la
 * contraseña no cuadran. La frase genérica de `unauthorized` («Tu sesión ha caducado. Vuelve a
 * iniciar sesión.») dejaba a quien se equivocaba tecleando mirando un mensaje que no describía
 * su problema y que le pedía hacer exactamente lo que estaba haciendo.
 *
 * Deliberadamente NO distingue cuál de los dos falla: decirlo confirmaría qué usuarios existen.
 */
export const LOGIN_INVALID_CREDENTIALS = "Usuario o contraseña incorrectos.";

/**
 * Traduce un error de la API a la frase que ve el usuario. Prioridad: código estable → código de
 * estado → frase genérica. NUNCA devuelve el mensaje técnico del backend: si un código no está en
 * el catálogo preferimos una frase genérica correcta a una precisa en inglés (y el test de
 * paridad `errorMessages.test.ts` existe para que ese caso no llegue a producción).
 */
export function messageForError(
  code: string | null,
  status: number | null,
): string {
  if (code && ERROR_MESSAGES[code]) return ERROR_MESSAGES[code];
  if (status && STATUS_MESSAGES[status]) return STATUS_MESSAGES[status];
  return FALLBACK;
}
