// Bounded, test-owned CPU/RAM/window workload; never targets other processes.
using System;
using System.Diagnostics;
using System.IO;
using System.Threading;
using System.Windows.Forms;

static class LiveWorkload {
    static volatile bool Done;
    static readonly byte[] Resident = new byte[16 * 1024 * 1024];
    [STAThread]
    static int Main(string[] args) {
        if (args.Length != 1) return 2;
        string stop = args[0];
        var worker = new Thread(delegate() {
            for (int i = 0; i < Resident.Length; i += 4096) Resident[i] = 17;
            var total = Stopwatch.StartNew();
            long value = 1;
            while (!Done && total.Elapsed.TotalSeconds < 120 && !File.Exists(stop)) {
                var pulse = Stopwatch.StartNew();
                while (pulse.ElapsedMilliseconds < 10) value = unchecked(value * 6364136223846793005L + 1);
                Thread.Sleep(40);
            }
            Done = true;
            GC.KeepAlive(value);
        });
        worker.IsBackground = true;
        Console.WriteLine("owned workload ready");
        worker.Start();
        using (var form = new Form()) {
            form.Text = "Owned MCP test workload";
            form.Width = 300; form.Height = 100;
            var timer = new System.Windows.Forms.Timer { Interval = 100 };
            timer.Tick += delegate { if (Done) form.Close(); };
            form.FormClosed += delegate { Done = true; };
            timer.Start(); Application.Run(form); timer.Dispose();
        }
        worker.Join(2000);
        return 0;
    }
}
