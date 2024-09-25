// ----------------- validators/user.xen
import lists { VafActuatorInterface }

validator InterfaceToColumn({
	ActuatorInterface VafActuatorInterface
}) {
	match @ActuatorInterface {
		"axles" => @[]
	}
}
