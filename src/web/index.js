let session = null
let pStatus = null
let selected = ""
let pollTimer = null
let revealRound = false
let sessionRequest = 0
let cacheExpanded = false

const $ = (id) => document.getElementById(id)

async function api(path, options) {
    const response = await fetch(path, options)
    const text = await response.text()

    let data

    try {
        data = text ? JSON.parse(text) : {}
    } catch {
        data = { error: text }
    }

    if (!response.ok) {
        throw data
    }

    return data
}

function post(path, body) {
    return api(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
    })
}

function escapeHtml(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
}

function setError(message = "") {
    $("error").textContent = message
    $("error").hidden = !message
}

function formatAge(seconds) {
    if (seconds === null || seconds === undefined) {
        return "unknown"
    }

    const days = Math.floor(seconds / 86400)
    const hours = Math.floor((seconds % 86400) / 3600)

    return days > 0 ? `${days}d ${hours}h` : `${hours}h`
}

function formatSize(bytes) {
    if (bytes < 1_000) {
        return `${bytes} B`
    }

    const units = ["kB", "MB", "GB"]
    let value = bytes / 1_000
    let unit = 0

    while (value >= 1_000 && unit < units.length - 1) {
        value /= 1_000
        unit += 1
    }

    let text = value.toFixed(1)

    if (text === "1000.0" && unit < units.length - 1) {
        value /= 1_000
        unit += 1
        text = value.toFixed(1)
    }

    return `${text} ${units[unit]}`
}

function copyButton(value, label) {
    if (!value) {
        return ""
    }

    return `<button data-copy="${escapeHtml(value)}">${label}</button>`
}

async function loadConfig() {
    const data = await api("/api/config")
    const select = $("municipality")

    for (const municipality of data.municipalities) {
        const option = document.createElement("option")
        option.value = municipality.code
        option.textContent = `${municipality.name} (${municipality.code})`
        select.appendChild(option)
    }

    if (data.default_municipality) {
        select.value = data.default_municipality
    }

    select.addEventListener("change", onMunicipalityChange)
    $("run").addEventListener("click", () => startRun(false))
    $("continue").addEventListener("click", () => {
        revealRound = true
        applySession()
    })
    $("adjust").addEventListener("click", startAdjust)
    $("show-completed").addEventListener("change", renderCells)
    document.addEventListener("click", onDocumentClick)

    await pollStatus()

    if (pStatus && pStatus.busy) {
        startPolling()
    }

    if (select.value) {
        await onMunicipalityChange()
    }
}

async function onMunicipalityChange() {
    sessionRequest += 1
    selected = $("municipality").value
    revealRound = false
    session = null
    $("round").hidden = true
    $("first-review").hidden = true
    $("adjust-details").hidden = true
    $("cells").innerHTML = ""
    $("adjust-cell").innerHTML = ""
    $("cache").innerHTML = ""

    setError()

    if (!selected) {
        $("run").disabled = true
        return
    }

    await loadSession()
    renderStatus()
}

function renderCache() {
    const cache = session.cache
    const cacheStatus =
        cache.entries.length === 0 || cache.entries.every((entry) => !entry.exists)
            ? "missing"
            : cache.entries.every((entry) => entry.exists && entry.fresh)
              ? "ok"
              : "partial"

    const entries = cache.entries
        .map((entry) => {
            const state = entry.exists
                ? `${formatSize(entry.size_bytes)}, updated ${formatAge(entry.age_seconds)} ago`
                : "missing"
            return `<li>${escapeHtml(entry.kind)}: <code>${escapeHtml(entry.path)}</code> (${escapeHtml(state)})</li>`
        })
        .join("")

    $("cache").innerHTML = `
        <p id="cache-toggle">Cache status: ${cacheStatus} (max. age: ${cache.max_age_days} days) &emsp; [click to ${cacheExpanded ? "hide" : "show"}]</p>
        <ul id="cache-list"${cacheExpanded ? "" : " hidden"}>${entries}</ul>
    `
}

async function loadSession() {
    if (!selected) {
        return
    }

    const request = ++sessionRequest

    try {
        const data = await api(`/api/session?municipality=${encodeURIComponent(selected)}`)

        if (request !== sessionRequest) {
            return
        }

        session = data
        applySession()
    } catch (error) {
        if (request === sessionRequest) {
            setError(error.error || "Failed to load the review round.")
        }
    }
}

function applySession() {
    if (!session) {
        return
    }

    renderCache()
    renderCells()

    const hasCells = session.cells.length > 0

    if (hasCells && !revealRound) {
        const reviewed = session.cells.filter((cell) => cell.buildings).length
        const done = session.cells.filter((cell) => cell.addresses).length
        const roundLabel = /^\d+$/.test(session.round_id)
            ? new Date(Number(session.round_id) * 1000).toLocaleString()
            : session.round_id
        $("round-summary").textContent =
            `Existing round ${roundLabel}: ${reviewed}/${session.cells.length} building cells reviewed, ${done}/${session.cells.length} address cells done.`
        $("round").hidden = false
        $("first-review").hidden = true
        $("adjust-details").hidden = true
        $("run").textContent = "Start new round"

        return
    }

    $("round").hidden = true
    $("first-review").hidden = !hasCells
    $("run").textContent = hasCells ? "Start new round" : "Run review"
}

function rowClass(cell) {
    if (cell.addresses) {
        return "row-done"
    }

    if (cell.skipped || cell.addresses_empty) {
        return "row-special"
    }

    return cell.buildings ? "row-ready" : ""
}

function cellTable(cells, action) {
    return `
        <div class="cell-table-scroll">
            <table>
                <thead>
                    <tr>
                        <th>Cell</th>
                        <th>Buildings</th>
                        <th>Addresses</th>
                        <th>Copy</th>
                        <th>Action</th>
                    </tr>
                </thead>
                <tbody>
                    ${cells
                        .map(
                            (cell) => `
                        <tr class="${rowClass(cell)}">
                            <td><code>${escapeHtml(cell.number)}</code></td>
                            <td>${cell.skipped ? "Skipped" : cell.buildings ? "Reviewed" : "Pending"}</td>
                            <td>${cell.addresses_empty ? "None" : cell.addresses ? "Done" : "Pending"}</td>
                            <td>
                                ${copyButton(cell.download_url, "OSM URL")}
                                ${cell.skipped ? "" : copyButton(cell.buildings_path, "buildings.osm")}
                                ${cell.addresses_empty ? "" : copyButton(cell.addresses_path, "addresses.osm")}
                            </td>
                            <td>${action(cell)}</td>
                        </tr>
                    `
                        )
                        .join("")}
                </tbody>
            </table>
        </div>
    `
}

function renderCells() {
    if (!session) {
        return
    }

    const visibleCells = $("show-completed").checked
        ? session.cells
        : session.cells.filter((cell) => !cell.addresses)

    $("cells").innerHTML = cellTable(visibleCells, (cell) => {
        if (cell.addresses) {
            return "Done"
        }

        if (cell.buildings || cell.skipped) {
            return `<button class="mark-done" data-cell="${escapeHtml(cell.number)}">Mark as done</button>`
        }

        return "<span>Run adjust first</span>"
    })

    const adjustable = session.cells.filter((cell) => !cell.skipped && !cell.buildings)
    const current = $("adjust-cell").value

    $("adjust-cell").innerHTML = adjustable
        .map(
            (cell) =>
                `<option value="${escapeHtml(cell.number)}">${escapeHtml(cell.number)}</option>`
        )
        .join("")
    $("adjust").disabled = adjustable.length === 0

    if (current && adjustable.some((cell) => cell.number === current)) {
        $("adjust-cell").value = current
    }
}

function hasAdjustableCells() {
    return Boolean(session && session.cells.some((cell) => !cell.skipped && !cell.buildings))
}

async function startRun(overwrite) {
    if (!selected) {
        setError("Select a municipality.")
        return
    }

    setError()

    try {
        await post("/api/run", { municipality: selected, overwrite })
        revealRound = true
        startPolling()
    } catch (error) {
        if (error.error === "overwrite_required" && !overwrite) {
            if (
                confirm(
                    "Review output already exists. Overwriting will delete manual review edits. Continue?"
                )
            ) {
                return startRun(true)
            }

            return
        }

        setError(error.error || "Run failed.")
    }
}

async function startAdjust() {
    const cell = $("adjust-cell").value

    if (!selected || !cell) {
        setError("Select a municipality and a cell.")
        return
    }

    setError()

    try {
        await post("/api/adjust", { municipality: selected, cell })
        revealRound = true
        startPolling()
    } catch (error) {
        setError(error.error || "Adjust failed.")
    }
}

async function markDone(cell) {
    if (!selected) {
        return
    }

    setError()

    try {
        await post("/api/cells/done", { municipality: selected, cell })
        await loadSession()
    } catch (error) {
        setError(error.error || "Failed to mark the cell done.")
    }
}

function startPolling() {
    if (!pollTimer) {
        pollTimer = setInterval(pollStatus, 800)
    }

    pollStatus()
}

async function pollStatus() {
    try {
        pStatus = await api("/api/status")
        renderStatus()

        if (!pStatus.busy) {
            if (pollTimer) {
                clearInterval(pollTimer)
                pollTimer = null
            }

            if (selected) {
                await loadSession()
            }
        }
    } catch (error) {
        setError(error.error || "Status check failed.")
    }
}

function renderStatus() {
    if (!pStatus) {
        return
    }

    if (pStatus.busy) {
        const progress = pStatus.progress || {}
        const percent = progress.percent
        const text =
            `${escapeHtml(progress.stage || "Working")}: ${escapeHtml(progress.message || "")}` +
            (percent === null || percent === undefined ? "" : ` (${percent}%)`)
        const adjusting = progress.stage === "Adjust"

        $("status").innerHTML = adjusting ? "" : `<p>${text}</p>`
        $("adjust-status").innerHTML = adjusting ? `<p>${text}</p>` : ""
        $("adjust-status").hidden = !adjusting
        $("run").disabled = true
        $("adjust").disabled = true
    } else {
        $("status").innerHTML = ""
        $("adjust-status").hidden = true
        $("run").disabled = !selected
        $("adjust").disabled = !hasAdjustableCells()
    }

    if (pStatus.error && pStatus.error_municipality === selected) {
        setError(pStatus.error)
    } else {
        setError()
    }

    if (pStatus.last_adjust && pStatus.last_adjust.municipality === selected) {
        const adjust = pStatus.last_adjust
        $("adjust-details").innerHTML = `
            <p>Cell <code>${escapeHtml(adjust.cell)}</code>: ${adjust.kept} kept buildings, ${adjust.removed} removed, ${adjust.placed} addresses kept on buildings, ${adjust.address_fixes} address fixes.</p>
            <p>Open the cell's <code>addresses.osm</code> path in JOSM for the address review.</p>
        `
        $("adjust-details").hidden = false
    } else {
        $("adjust-details").hidden = true
    }
}

async function onDocumentClick(event) {
    const cacheToggle = event.target.closest("#cache-toggle")

    if (cacheToggle) {
        cacheExpanded = !cacheExpanded
        renderCache()
        return
    }

    const copy = event.target.closest("[data-copy]")

    if (copy) {
        await copyText(copy.dataset.copy, copy)
        return
    }

    const done = event.target.closest(".mark-done")

    if (done) {
        await markDone(done.dataset.cell)
    }
}

async function copyText(text, button) {
    try {
        await navigator.clipboard.writeText(text)
    } catch (error) {
        setError(error.message || "Failed to copy to clipboard.")
        return
    }

    button.classList.add("copied")

    setTimeout(() => {
        button.classList.remove("copied")
    }, 1200)
}

loadConfig().catch((error) => setError(error.error || "Failed to load the web interface."))
