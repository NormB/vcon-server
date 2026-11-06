// Command vcon-client works through the HTTP API with the Go standard
// library only: it stores a vCon, finds it in the list, reads it back,
// stores and reads back a gzipped vCon, pages through both with a cursor,
// replaces the first under If-Match, places and releases a legal hold,
// finds its own create in the audit log, lists the tenants with an
// operator key, reads the vCon over MCP, and deletes both.
//
// Set VCON_URL (https://vcon.example.com) and four keys: VCON_KEY (the
// write, read and delete scopes), VCON_LEGAL_KEY (legal), VCON_AUDIT_KEY
// (audit) and VCON_OPERATOR_KEY (an operator key). For a certificate your
// system does not trust, set VCON_CA (its PEM file).
package main

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strings"
)

// marked is how vcon_get returns a vCon's text.
const marked = "\u27e6untrusted-vcon-data\u27e7%s\u27e6/untrusted-vcon-data\u27e7"

type client struct {
	url, key string
	http     *http.Client
}

// answer is one answer: its status, headers and body.
type answer struct {
	status  int
	headers http.Header
	body    []byte
}

// call sends one request with the client's key and the given headers, a
// name then a value. An answer of 400 or more is a problem document,
// returned as an error with its type.
func (c client) call(method, path string, body []byte, headers ...string) (answer, error) {
	request, err := http.NewRequest(method, c.url+path, bytes.NewReader(body))
	if err != nil {
		return answer{}, err
	}
	request.Header.Set("Authorization", "Bearer "+c.key)
	for i := 0; i+1 < len(headers); i += 2 {
		request.Header.Set(headers[i], headers[i+1])
	}
	got, err := c.http.Do(request)
	if err != nil {
		return answer{}, err
	}
	defer got.Body.Close()
	read, err := io.ReadAll(got.Body)
	if err != nil {
		return answer{}, err
	}
	a := answer{got.StatusCode, got.Header, read}
	if got.StatusCode >= 400 {
		var problem struct {
			Type string `json:"type"`
		}
		_ = json.Unmarshal(read, &problem)
		return a, fmt.Errorf("%s %s: %d %s", method, path, got.StatusCode, problem.Type)
	}
	return a, nil
}

// as is the same client with another key.
func (c client) as(key string) client {
	c.key = key
	return c
}

// get sends a GET and decodes the JSON answer into v.
func (c client) get(path string, v any) error {
	a, err := c.call("GET", path, nil)
	if err != nil {
		return err
	}
	return json.Unmarshal(a.body, v)
}

// mcp posts one JSON-RPC message to /mcp and decodes the answer's result
// into result, unless it is nil (a notification).
func (c client) mcp(message map[string]any, result any) error {
	message["jsonrpc"] = "2.0"
	body, err := json.Marshal(message)
	if err != nil {
		return err
	}
	a, err := c.call("POST", "/mcp", body,
		"Content-Type", "application/json", "Accept", "application/json, text/event-stream")
	if err != nil || result == nil {
		return err
	}
	var rpc struct {
		Result json.RawMessage `json:"result"`
		Error  any             `json:"error"`
	}
	if err := json.Unmarshal(a.body, &rpc); err != nil {
		return err
	}
	if rpc.Error != nil {
		return fmt.Errorf("%v answered %v", message["method"], rpc.Error)
	}
	return json.Unmarshal(rpc.Result, result)
}

// newUUID is a random (version 4) uuid in RFC 9562 text form.
func newUUID() string {
	var b [16]byte
	rand.Read(b[:])
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

// encode is a vCon's JSON text; encoding/json writes a map's keys sorted.
func encode(id, subject string) ([]byte, error) {
	return json.Marshal(map[string]any{
		"created_at": "2026-10-25T12:00:00Z",
		"parties":    []any{},
		"subject":    subject,
		"uuid":       id,
	})
}

func run() error {
	roots, err := x509.SystemCertPool()
	if err != nil {
		return err
	}
	if path := os.Getenv("VCON_CA"); path != "" {
		pem, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if !roots.AppendCertsFromPEM(pem) {
			return fmt.Errorf("no certificate in %s", path)
		}
	}
	c := client{
		url:  os.Getenv("VCON_URL"),
		key:  os.Getenv("VCON_KEY"),
		http: &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}}},
	}
	id := newUUID()
	vcon, err := encode(id, "client example")
	if err != nil {
		return err
	}

	// Store: 201, with the new version in ETag.
	stored, err := c.call("POST", "/vcons", vcon, "Content-Type", "application/vcon")
	if err != nil {
		return err
	}
	if stored.status != 201 {
		return fmt.Errorf("a store answered %d, not 201", stored.status)
	}
	firstTag := stored.headers.Get("ETag")
	fmt.Println("stored", id)

	// List: the newest vCon comes first.
	type listing struct {
		Vcons []struct {
			UUID     string `json:"uuid"`
			StoredAt string `json:"stored_at"`
		} `json:"vcons"`
		Next *string `json:"next"`
	}
	var page listing
	if err := c.get("/vcons?limit=10", &page); err != nil {
		return err
	}
	storedAt := ""
	for _, v := range page.Vcons {
		if v.UUID == id {
			storedAt = v.StoredAt
		}
	}
	if storedAt == "" {
		return fmt.Errorf("the stored vCon is not in the list")
	}
	fmt.Println("listed", id)

	// Read: the body comes back exactly as stored.
	read, err := c.call("GET", "/vcons/"+id, nil)
	if err != nil {
		return err
	}
	if !bytes.Equal(read.body, vcon) {
		return fmt.Errorf("the body read differs from the one stored")
	}
	fmt.Println("read", id, len(read.body), "bytes")

	// Gzip: a gzipped body is stored as sent, and read back with its media type.
	gzipID := newUUID()
	plain, err := encode(gzipID, "gzip example")
	if err != nil {
		return err
	}
	var sent bytes.Buffer
	zipper := gzip.NewWriter(&sent)
	if _, err := zipper.Write(plain); err != nil {
		return err
	}
	if err := zipper.Close(); err != nil {
		return err
	}
	if _, err := c.call("POST", "/vcons", sent.Bytes(), "Content-Type", "application/vcon+gzip"); err != nil {
		return err
	}
	read, err = c.call("GET", "/vcons/"+gzipID, nil)
	if err != nil {
		return err
	}
	if !bytes.Equal(read.body, sent.Bytes()) {
		return fmt.Errorf("the gzipped body read differs from the one stored")
	}
	if read.headers.Get("Content-Type") != "application/vcon+gzip" {
		return fmt.Errorf("a gzipped vCon was not read back as application/vcon+gzip")
	}
	fmt.Println("gzip", gzipID, "read back as application/vcon+gzip")

	// Page: one vCon a page, from the first one's stored_at on, following
	// next until it is null. Each of the two must be seen exactly once.
	query := "/vcons?limit=1&since=" + url.QueryEscape(storedAt)
	seen := map[string]int{}
	listed, pages := 0, 0
	for path := query; ; {
		var page listing
		if err := c.get(path, &page); err != nil {
			return err
		}
		pages++
		for _, v := range page.Vcons {
			seen[v.UUID]++
			listed++
		}
		if page.Next == nil {
			break
		}
		path = query + "&cursor=" + url.QueryEscape(*page.Next)
	}
	for _, each := range []string{id, gzipID} {
		if seen[each] != 1 {
			return fmt.Errorf("the pages did not list %s exactly once", each)
		}
	}
	fmt.Println("paged", listed, "vCons in", pages, "pages of 1")

	// Replace: If-Match names the current version; the answer has the new
	// one. The first version's tag is then stale, and refused with 412.
	replaced, err := encode(id, "client example, replaced")
	if err != nil {
		return err
	}
	put, err := c.call("PUT", "/vcons/"+id, replaced, "If-Match", firstTag, "Content-Type", "application/vcon")
	if err != nil {
		return err
	}
	tag := put.headers.Get("ETag")
	if tag == "" || tag == firstTag {
		return fmt.Errorf("a replace did not answer a new ETag")
	}
	stale, err := c.call("PUT", "/vcons/"+id, replaced, "If-Match", firstTag, "Content-Type", "application/vcon")
	if stale.status != 412 || !strings.HasSuffix(fmt.Sprint(err), "412 https://vcon-server.com/problems/precondition-failed") {
		return fmt.Errorf("a stale If-Match answered %d, not 412 precondition-failed", stale.status)
	}
	fmt.Println("replaced " + id + ", a stale If-Match refused with 412")

	// Legal hold: placed and released with the legal key, 204 each.
	for _, method := range []string{"PUT", "DELETE"} {
		held, err := c.as(os.Getenv("VCON_LEGAL_KEY")).call(method, "/legal/vcons/"+id+"/hold", nil)
		if err != nil {
			return err
		}
		if held.status != 204 {
			return fmt.Errorf("%s hold answered %d, not 204", method, held.status)
		}
	}
	fmt.Println("held", id, "and released")

	// Audit: the tenant's log, read with the audit key, holds this
	// program's create, made by VCON_KEY, whose id and tenant GET /key gives.
	var me struct {
		ID     string `json:"id"`
		Tenant string `json:"tenant"`
	}
	if err := c.get("/key", &me); err != nil {
		return err
	}
	auditor := c.as(os.Getenv("VCON_AUDIT_KEY"))
	for after, found := 0, false; !found; {
		page, err := auditor.call("GET", fmt.Sprintf("/audit?after=%d&limit=1000", after), nil)
		if err != nil {
			return err
		}
		records := 0
		for lines := bufio.NewScanner(bytes.NewReader(page.body)); lines.Scan(); records++ {
			var r struct {
				Seq    int     `json:"seq"`
				Action string  `json:"action"`
				Actor  string  `json:"actor"`
				Target *string `json:"target"`
				Status int     `json:"status"`
			}
			if err := json.Unmarshal(lines.Bytes(), &r); err != nil {
				return err
			}
			found = found || r.Action == "create" && r.Target != nil && *r.Target == id &&
				r.Actor == me.ID && r.Status == 201
			after = r.Seq
		}
		if !found && records < 1000 {
			return fmt.Errorf("the audit log has no create of %s", id)
		}
	}
	fmt.Println("audit create", id, "by", me.ID)

	// Admin: the operator key lists every tenant, this key's among them.
	var tenants []struct {
		ID string `json:"id"`
	}
	if err := c.as(os.Getenv("VCON_OPERATOR_KEY")).get("/admin/tenants", &tenants); err != nil {
		return err
	}
	has := false
	for _, t := range tenants {
		has = has || t.ID == me.Tenant
	}
	if !has {
		return fmt.Errorf("the admin tenant list does not have %s", me.Tenant)
	}
	fmt.Println("admin tenant", me.Tenant, "listed")

	// MCP: initialize, tools/list, and vcon_get, which answers the replaced
	// vCon's text between the untrusted-data markers, and its version.
	var init struct {
		ProtocolVersion string `json:"protocolVersion"`
		ServerInfo      struct {
			Name string `json:"name"`
		} `json:"serverInfo"`
	}
	if err := c.mcp(map[string]any{"id": 1, "method": "initialize", "params": map[string]any{
		"protocolVersion": "2025-06-18",
		"capabilities":    map[string]any{},
		"clientInfo":      map[string]any{"name": "vcon-client", "version": "1"},
	}}, &init); err != nil {
		return err
	}
	if init.ProtocolVersion != "2025-06-18" {
		return fmt.Errorf("initialize did not agree to 2025-06-18")
	}
	if err := c.mcp(map[string]any{"method": "notifications/initialized"}, nil); err != nil {
		return err
	}
	var tools struct {
		Tools []struct {
			Name string `json:"name"`
		} `json:"tools"`
	}
	if err := c.mcp(map[string]any{"id": 2, "method": "tools/list"}, &tools); err != nil {
		return err
	}
	hasGet := false
	for _, t := range tools.Tools {
		hasGet = hasGet || t.Name == "vcon_get"
	}
	if !hasGet {
		return fmt.Errorf("tools/list has no vcon_get")
	}
	var got struct {
		IsError           bool `json:"isError"`
		StructuredContent struct {
			Version string `json:"version"`
			Vcon    string `json:"vcon"`
		} `json:"structuredContent"`
	}
	if err := c.mcp(map[string]any{"id": 3, "method": "tools/call", "params": map[string]any{
		"name": "vcon_get", "arguments": map[string]any{"uuid": id},
	}}, &got); err != nil {
		return err
	}
	if got.IsError {
		return fmt.Errorf("vcon_get answered an error")
	}
	if `"`+got.StructuredContent.Version+`"` != tag {
		return fmt.Errorf("vcon_get answered another version")
	}
	if got.StructuredContent.Vcon != fmt.Sprintf(marked, replaced) {
		return fmt.Errorf("vcon_get answered another text")
	}
	fmt.Printf("mcp vcon_get %s from %s, %d tools listed\n", id, init.ServerInfo.Name, len(tools.Tools))

	// Delete: 204, and each vCon is gone (410) from then on.
	for _, each := range []string{id, gzipID} {
		deleted, err := c.call("DELETE", "/vcons/"+each, nil)
		if err != nil {
			return err
		}
		if deleted.status != 204 {
			return fmt.Errorf("a delete answered %d, not 204", deleted.status)
		}
		if gone, _ := c.call("GET", "/vcons/"+each, nil); gone.status != 410 {
			return fmt.Errorf("a deleted vCon answered %d, not 410", gone.status)
		}
	}
	fmt.Println("deleted", id, gzipID)
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
