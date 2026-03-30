-- forge.nvim — Neovim plugin for forge code review comments.
--
-- Usage: require("forge").setup({ ... })
-- Requires `forge` CLI on $PATH and a review worktree context.

local M = {}

M.config = {
  keymaps = {
    comment = "<leader>fc",
    reply = "<leader>fr",
    resolve = "<leader>fR",
    list = "<leader>fl",
  },
  window = {
    width = 0.6,
    height = 0.4,
    border = "rounded",
  },
}

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

local function git_root()
  local out = vim.fn.system("git rev-parse --show-toplevel")
  if vim.v.shell_error ~= 0 then
    return nil
  end
  return vim.trim(out)
end

local function relative_path(bufnr)
  local abs = vim.api.nvim_buf_get_name(bufnr or 0)
  local root = git_root()
  if not root then
    return nil
  end
  if abs:sub(1, #root) == root then
    return abs:sub(#root + 2)
  end
  return abs
end

local function blob_oid(path)
  -- Escape the entire HEAD:path as a single shell argument.
  local out = vim.fn.system({ "git", "rev-parse", "HEAD:" .. path })
  if vim.v.shell_error ~= 0 then
    return nil
  end
  local oid = vim.trim(out)
  if not oid:match("^%x+$") then
    return nil
  end
  return oid
end

-- ---------------------------------------------------------------------------
-- Float window
-- ---------------------------------------------------------------------------

local function close_win(win)
  if vim.api.nvim_win_is_valid(win) then
    vim.api.nvim_win_close(win, true)
  end
end

local function open_float(title)
  local w = M.config.window
  local width = math.floor(vim.o.columns * w.width)
  local height = math.floor(vim.o.lines * w.height)
  local row = math.floor((vim.o.lines - height) / 2)
  local col = math.floor((vim.o.columns - width) / 2)

  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].filetype = "markdown"
  vim.bo[buf].bufhidden = "wipe"

  local win = vim.api.nvim_open_win(buf, true, {
    relative = "editor",
    width = width,
    height = height,
    row = row,
    col = col,
    style = "minimal",
    border = w.border,
    title = title,
    title_pos = "center",
  })

  vim.cmd("startinsert")
  return buf, win
end

-- ---------------------------------------------------------------------------
-- Submit
-- ---------------------------------------------------------------------------

local function submit(buf, win, argv)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local body = vim.trim(table.concat(lines, "\n"))
  if body == "" then
    vim.notify("forge: empty comment, cancelled", vim.log.levels.WARN)
    close_win(win)
    return
  end

  local tmpfile = vim.fn.tempname()
  vim.fn.writefile(vim.split(body, "\n", { plain = true }), tmpfile)

  local cmd = vim.list_extend(vim.list_slice(argv), { "-f", tmpfile })
  close_win(win)

  vim.fn.jobstart(cmd, {
    on_exit = function(_, code)
      vim.schedule(function()
        vim.fn.delete(tmpfile)
        if code == 0 then
          vim.notify("forge: comment submitted", vim.log.levels.INFO)
        else
          vim.notify("forge: comment failed (exit " .. code .. ")", vim.log.levels.ERROR)
        end
      end)
    end,
  })
end

local function bind_float(buf, win, argv)
  local opts = { buffer = buf, silent = true }
  vim.keymap.set("n", "ZZ", function() submit(buf, win, argv) end, opts)
  vim.keymap.set("n", "q", function() close_win(win) end, opts)
  vim.keymap.set("n", "<Esc>", function() close_win(win) end, opts)
end

-- ---------------------------------------------------------------------------
-- Actions
-- ---------------------------------------------------------------------------

function M.add_comment(visual)
  local path = relative_path()
  if not path then
    vim.notify("forge: not in a git repository", vim.log.levels.ERROR)
    return
  end

  local oid = blob_oid(path)
  if not oid then
    vim.notify("forge: file not tracked by git", vim.log.levels.ERROR)
    return
  end

  local start_line, end_line
  if visual then
    start_line = vim.fn.line("'<")
    end_line = vim.fn.line("'>")
  else
    start_line = vim.fn.line(".")
    end_line = start_line
  end

  local range = start_line .. "-" .. end_line
  local title = string.format(" Comment on %s:%s ", path, range)
  local argv = {
    "forge", "comment", "add",
    "--anchor", oid,
    "--anchor-path", path,
    "--range", range,
  }

  local buf, win = open_float(title)
  bind_float(buf, win, argv)
end

function M.reply()
  vim.ui.input({ prompt = "Reply to comment OID: " }, function(oid)
    if not oid or oid == "" then return end
    local title = string.format(" Reply to %s ", oid)
    local argv = { "forge", "comment", "reply", "--to", oid }
    local buf, win = open_float(title)
    bind_float(buf, win, argv)
  end)
end

function M.resolve()
  vim.ui.input({ prompt = "Resolve thread OID: " }, function(oid)
    if not oid or oid == "" then return end
    vim.fn.jobstart({ "forge", "comment", "resolve", "--thread", oid }, {
      on_exit = function(_, code)
        vim.schedule(function()
          if code == 0 then
            vim.notify("forge: thread resolved", vim.log.levels.INFO)
          else
            vim.notify("forge: resolve failed (exit " .. code .. ")", vim.log.levels.ERROR)
          end
        end)
      end,
    })
  end)
end

function M.list_comments()
  vim.fn.jobstart({ "forge", "--json", "comment", "list" }, {
    stdout_buffered = true,
    on_stdout = function(_, data)
      vim.schedule(function()
        local text = table.concat(data, "\n")
        if vim.trim(text) == "" then
          vim.notify("forge: no comments", vim.log.levels.INFO)
          return
        end
        local ok, comments = pcall(vim.json.decode, text)
        if not ok or not comments then
          vim.notify("forge: failed to parse comments", vim.log.levels.ERROR)
          return
        end
        local items = {}
        for _, c in ipairs(comments) do
          local short_oid = c.oid and c.oid:sub(1, 12) or "?"
          local anchor_info = ""
          if c.anchor then
            local a = c.anchor
            if a.Object then
              local p = a.Object.path or ""
              local r = a.Object.range or ""
              anchor_info = p .. (r ~= "" and (":" .. r) or "")
            end
          end
          local first_line = (c.body or ""):match("^([^\n]*)")
          table.insert(items, {
            text = string.format("[%s] %s %s", short_oid, anchor_info, first_line),
            filename = anchor_info ~= "" and anchor_info:match("^[^:]*") or "",
            lnum = anchor_info:match(":(%d+)") or 0,
          })
        end
        vim.fn.setloclist(0, items, "r")
        vim.cmd("lopen")
      end)
    end,
  })
end

-- ---------------------------------------------------------------------------
-- Setup
-- ---------------------------------------------------------------------------

function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", M.config, opts or {})

  if vim.fn.executable("forge") ~= 1 then
    vim.notify("forge: `forge` binary not found on $PATH", vim.log.levels.WARN)
    return
  end

  local km = M.config.keymaps

  vim.keymap.set("n", km.comment, function() M.add_comment(false) end, { desc = "Forge: comment" })
  vim.keymap.set("v", km.comment, function()
    vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes("<Esc>", true, false, true), "x", false)
    M.add_comment(true)
  end, { desc = "Forge: comment (visual)" })
  vim.keymap.set("n", km.reply, M.reply, { desc = "Forge: reply" })
  vim.keymap.set("n", km.resolve, M.resolve, { desc = "Forge: resolve" })
  vim.keymap.set("n", km.list, M.list_comments, { desc = "Forge: list comments" })
end

return M
