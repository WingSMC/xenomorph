# Unimplemented design sketches

These are proposals, not accepted Xenomorph syntax. See the
[language reference](GRAMMAR.md) for the implemented grammar.

Earlier ideas also included collection concatenation and set operations,
runtime `$field` references, boolean validator operators, range validators,
and `eq`, `neq`, `in`, `empty`, `only`, and `exclude` annotations. They are not
implemented.

The following validator declaration is a historical sketch:

```text
// ----------------- validators/user.xen
import lists { VafActuatorInterface }

validator InterfaceToColumn({
	ActuatorInterface VafActuatorInterface
}) {
	match @ActuatorInterface {
		"axles" => @[]
	}
}
```
