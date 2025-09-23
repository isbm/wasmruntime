// hello.go
package main

import (
	"encoding/json"
	"fmt"
	"os"
)

func main() {
	msg := map[string]any{
		"ok":     true,
		"msg":    "hello world from Go (Normal and Tiny)",
		"result": "Looks OK",
	}

	s := "Some text here.\n"
	err := os.WriteFile("golang-output.txt", []byte(s), 0644)
	if err != nil {
		msg["ok"] = false
		msg["result"] = err.Error()
	}

	data, err := json.Marshal(msg)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}

	fmt.Println(string(data))
}
