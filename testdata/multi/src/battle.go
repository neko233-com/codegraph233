package battle

type Unit struct{ HP int }

func (u *Unit) Hit(d int) int { return u.damage(d) }

func (u *Unit) damage(d int) int { u.HP -= d; return u.HP }
