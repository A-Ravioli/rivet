//! Bus models in loopback: the kit master and the kit slave share the same
//! signals, so each protocol's driver and responder are checked against
//! each other with random wait states.

use rivet_core::{random, LogicVec, TimeExt};
use rivet_kit::bus::axi::AxiBurst;
use rivet_kit::bus::axi_lite::{axi_lite_monitor, AxiLiteLog};
use rivet_kit::bus::{
    Apb, ApbMaster, ApbSlave, Avalon, AvalonMaster, AvalonSlave, Axi, AxiLite, AxiLiteMaster, AxiLiteSlave, AxiMaster,
    AxiSlave, Axis, AxisBeat, AxisSink, AxisSource, Resp, Wishbone, WishboneMaster, WishboneSlave,
};
use rivet_kit::{Backpressure, Memory};
use rivet_mock::{run_test, Design};

fn design(signals: &[(&str, u32)]) -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    for (n, w) in signals {
        let h = d.logic(n, *w);
        d.init(h, LogicVec::from_u64(*w, 0));
    }
    d
}

fn axil_design() -> Design {
    design(&[
        ("axil_awaddr", 32),
        ("axil_awvalid", 1),
        ("axil_awready", 1),
        ("axil_wdata", 32),
        ("axil_wstrb", 4),
        ("axil_wvalid", 1),
        ("axil_wready", 1),
        ("axil_bresp", 2),
        ("axil_bvalid", 1),
        ("axil_bready", 1),
        ("axil_araddr", 32),
        ("axil_arvalid", 1),
        ("axil_arready", 1),
        ("axil_rdata", 32),
        ("axil_rresp", 2),
        ("axil_rvalid", 1),
        ("axil_rready", 1),
    ])
}

#[test]
fn axi_lite_master_and_slave_agree() {
    random::set_base_seed(11);
    run_test(axil_design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let bus = AxiLite::find(&dut, clk, "axil_")?;
        let mem = Memory::new();
        let log = AxiLiteLog::default();
        let _mon = axi_lite_monitor(bus.clone(), log.clone());
        let _slave = AxiLiteSlave::new(bus.clone(), mem.clone())
            .backpressure(Backpressure::random(0, 3), Backpressure::random(0, 2))
            .error_range(0x1000, 0x2000)
            .run();
        let mut m = AxiLiteMaster::new(bus);
        clk.rising_edge().await;
        for i in 0..8u64 {
            m.write(i * 4, 0x1000_0000 + i).await?;
        }
        assert_eq!(mem.read_u32(0x14), 0x1000_0005);
        for i in 0..8u64 {
            assert_eq!(m.read(i * 4).await?, 0x1000_0000 + i, "readback {i}");
        }
        assert_eq!(m.write_strb(0x14, 0xffff_ffff, 0b0011).await, Resp::Okay);
        assert_eq!(mem.read_u32(0x14), 0x1000_ffff);
        assert_eq!(m.write_strb(0x1004, 1, 0xf).await, Resp::SlvErr);
        assert!(m.write(0x1008, 1).await.is_err());
        assert_eq!(m.read_resp(0x1004).await, (0, Resp::SlvErr));
        assert_eq!(mem.read_u32(0x1004), 0, "error range untouched");
        // Let the monitor observe the final handshake.
        clk.rising_edge().await;
        let writes = log.writes.borrow().clone();
        assert_eq!(writes.len(), 11);
        assert_eq!(writes[8], (0x14, 0xffff_ffff, 0b0011));
        assert_eq!(writes[9], (0x1004, 1, 0xf), "errored writes still complete on the bus");
        assert_eq!(log.reads.borrow().len(), 9);
        assert_eq!(log.reads.borrow()[3], (0xc, 0x1000_0003));
        Ok(())
    })
    .unwrap();
}

fn axi_design() -> Design {
    design(&[
        ("axi_awid", 4),
        ("axi_awaddr", 32),
        ("axi_awlen", 8),
        ("axi_awsize", 3),
        ("axi_awburst", 2),
        ("axi_awvalid", 1),
        ("axi_awready", 1),
        ("axi_wdata", 32),
        ("axi_wstrb", 4),
        ("axi_wlast", 1),
        ("axi_wvalid", 1),
        ("axi_wready", 1),
        ("axi_bid", 4),
        ("axi_bresp", 2),
        ("axi_bvalid", 1),
        ("axi_bready", 1),
        ("axi_arid", 4),
        ("axi_araddr", 32),
        ("axi_arlen", 8),
        ("axi_arsize", 3),
        ("axi_arburst", 2),
        ("axi_arvalid", 1),
        ("axi_arready", 1),
        ("axi_rid", 4),
        ("axi_rdata", 32),
        ("axi_rresp", 2),
        ("axi_rlast", 1),
        ("axi_rvalid", 1),
        ("axi_rready", 1),
        ("axi_awprot", 3),
    ])
}

#[test]
fn axi_bursts_narrow_transfers_ids_and_errors() {
    random::set_base_seed(12);
    run_test(axi_design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let bus = Axi::find(&dut, clk, "axi_")?;
        let mem = Memory::new();
        let _slave = AxiSlave::new(bus.clone(), mem.clone())
            .backpressure(Backpressure::random(0, 2), Backpressure::random(0, 2))
            .error_range(0x8000, 0x9000)
            .run();
        let mut m = AxiMaster::new(bus);
        m.id = 5;
        clk.rising_edge().await;
        // INCR burst of four 32-bit beats.
        let data = [0x11u64, 0x22, 0x33, 0x44];
        assert_eq!(m.write_burst(0x100, &data, 2, AxiBurst::Incr).await?, Resp::Okay);
        for (i, d) in data.iter().enumerate() {
            assert_eq!(mem.read_u32(0x100 + 4 * i as u64), *d as u32);
        }
        let (rd, resp) = m.read_burst(0x100, 3, 2, AxiBurst::Incr).await?;
        assert_eq!((rd, resp), (data.to_vec(), Resp::Okay));
        // WRAP burst starting mid-block: 0x118, 0x11c, 0x110, 0x114.
        assert_eq!(m.write_burst(0x118, &[1, 2, 3, 4], 2, AxiBurst::Wrap).await?, Resp::Okay);
        assert_eq!([mem.read_u32(0x110), mem.read_u32(0x114), mem.read_u32(0x118), mem.read_u32(0x11c)], [3, 4, 1, 2]);
        let (rd, _) = m.read_burst(0x118, 3, 2, AxiBurst::Wrap).await?;
        assert_eq!(rd, [1, 2, 3, 4]);
        // FIXED: every beat hits the same address; the last one sticks.
        assert_eq!(m.write_burst(0x200, &[7, 8, 9], 2, AxiBurst::Fixed).await?, Resp::Okay);
        assert_eq!(mem.read_u32(0x200), 9);
        assert_eq!(mem.read_u32(0x204), 0);
        // Narrow byte transfers on the 32-bit bus land on the right lanes.
        assert_eq!(m.write_burst(0x301, &[0xaa, 0xbb, 0xcc], 0, AxiBurst::Incr).await?, Resp::Okay);
        assert_eq!(mem.read_u32(0x300), 0xccbb_aa00);
        let (rd, _) = m.read_burst(0x301, 2, 0, AxiBurst::Incr).await?;
        assert_eq!(rd, [0xaa, 0xbb, 0xcc]);
        // Single-beat helpers and error responses.
        m.write(0x400, 0xdead_beef).await?;
        assert_eq!(m.read(0x400).await?, 0xdead_beef);
        assert_eq!(m.write_burst(0x8000, &[1], 2, AxiBurst::Incr).await?, Resp::SlvErr);
        assert!(m.read(0x8004).await.is_err());
        assert_eq!(mem.read_u32(0x8000), 0);
        Ok(())
    })
    .unwrap();
}

fn axis_design() -> Design {
    design(&[("s_tvalid", 1), ("s_tready", 1), ("s_tdata", 32), ("s_tkeep", 4), ("s_tlast", 1), ("s_tuser", 2)])
}

#[test]
fn axi_stream_packets_survive_gaps_and_backpressure() {
    random::set_base_seed(13);
    run_test(axis_design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let bus = Axis::find(&dut, clk, "s_")?;
        let counter = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let _count = rivet_core::spawn(rivet_kit::bus::axis::axis_count_transfers(bus.clone(), counter.clone()));
        let mut src = AxisSource::new(bus.clone()).gaps(Backpressure::random(0, 2));
        let mut sink = AxisSink::new(bus).backpressure(Backpressure::random(0, 3));
        let packets: Vec<Vec<u8>> = vec![vec![1], (0..9).collect(), (10..18).collect(), vec![]];
        let expected = packets.clone();
        let sender = rivet_core::spawn(async move {
            for p in &packets {
                if p.is_empty() {
                    src.send_beat(&AxisBeat { data: 0, keep: 0, last: true, user: 3 }).await;
                } else {
                    src.send_packet(p).await;
                }
            }
        });
        for (i, p) in expected.iter().enumerate() {
            let got = sink.recv_packet().await;
            assert_eq!(&got, p, "packet {i}");
        }
        let _ = sender.await;
        // 1 + 3 + 2 + 1 beats.
        clk.rising_edge().await;
        assert_eq!(counter.get(), 7);
        Ok(())
    })
    .unwrap();
}

fn apb_design() -> Design {
    design(&[
        ("apb_psel", 1),
        ("apb_penable", 1),
        ("apb_pwrite", 1),
        ("apb_paddr", 16),
        ("apb_pwdata", 32),
        ("apb_prdata", 32),
        ("apb_pready", 1),
        ("apb_pslverr", 1),
        ("apb_pstrb", 4),
    ])
}

#[test]
fn apb_transfers_with_wait_states() {
    random::set_base_seed(14);
    run_test(apb_design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let bus = Apb::find(&dut, clk, "apb_")?;
        let mem = Memory::new();
        let _slave = ApbSlave::new(bus.clone(), mem.clone())
            .backpressure(Backpressure::random(0, 3))
            .error_range(0xf000, 0x1_0000)
            .run();
        let mut m = ApbMaster::new(bus);
        clk.rising_edge().await;
        let t0 = rivet_core::now();
        m.write(0x10, 0xcafe_f00d).await?;
        assert_eq!(mem.read_u32(0x10), 0xcafe_f00d);
        assert_eq!(m.read(0x10).await?, 0xcafe_f00d);
        assert!(!m.write_strb(0x10, 0, 0b1000).await);
        assert_eq!(m.read(0x10).await?, 0x00fe_f00d);
        assert!(m.write(0xf004, 1).await.is_err());
        assert!(m.read(0xf004).await.is_err());
        assert_eq!(mem.read_u32(0xf004), 0);
        // Six transfers of at least 2 cycles each (10ns).
        assert!(rivet_core::now() - t0 >= 120);
        Ok(())
    })
    .unwrap();
}

fn avalon_design(pipelined: bool) -> Design {
    let mut s = vec![
        ("av_address", 32),
        ("av_read", 1),
        ("av_write", 1),
        ("av_readdata", 32),
        ("av_writedata", 32),
        ("av_byteenable", 4),
        ("av_waitrequest", 1),
    ];
    if pipelined {
        s.push(("av_readdatavalid", 1));
    }
    design(&s)
}

#[test]
fn avalon_simple_and_pipelined() {
    for pipelined in [false, true] {
        random::set_base_seed(15);
        run_test(avalon_design(pipelined), move |dut| async move {
            let clk = dut.signal("clk")?;
            let bus = Avalon::find(&dut, clk, "av_")?;
            assert_eq!(bus.readdatavalid.is_some(), pipelined);
            let mem = Memory::new();
            let _slave = AvalonSlave::new(bus.clone(), mem.clone()).backpressure(Backpressure::random(0, 3)).run();
            let mut m = AvalonMaster::new(bus);
            clk.rising_edge().await;
            for i in 0..4u64 {
                m.write(0x40 + 4 * i, 0xa000 + i).await;
            }
            assert_eq!(mem.read_u32(0x48), 0xa002);
            for i in 0..4u64 {
                assert_eq!(m.read(0x40 + 4 * i).await, 0xa000 + i, "pipelined={pipelined} word {i}");
            }
            m.write_be(0x40, 0xffff_ffff, 0b0001).await;
            assert_eq!(m.read(0x40).await, 0xa0ff);
            Ok(())
        })
        .unwrap();
    }
}

fn wb_design() -> Design {
    design(&[
        ("wb_cyc", 1),
        ("wb_stb", 1),
        ("wb_we", 1),
        ("wb_adr", 32),
        ("wb_dat_w", 32),
        ("wb_dat_r", 32),
        ("wb_ack", 1),
        ("wb_sel", 4),
        ("wb_err", 1),
    ])
}

#[test]
fn wishbone_classic_cycles() {
    random::set_base_seed(16);
    run_test(wb_design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let bus = Wishbone::find(&dut, clk, "wb_")?;
        let mem = Memory::new();
        let _slave = WishboneSlave::new(bus.clone(), mem.clone())
            .backpressure(Backpressure::random(0, 2))
            .error_range(0x100, 0x200)
            .run();
        let mut m = WishboneMaster::new(bus);
        clk.rising_edge().await;
        m.write(0x0, 0x0102_0304).await?;
        m.write_sel(0x0, 0xffff_ffff, 0b0100).await?;
        assert_eq!(m.read(0x0).await?, 0x01ff_0304);
        assert!(m.write(0x104, 1).await.is_err());
        assert!(m.read(0x104).await.is_err());
        assert_eq!(mem.read_u32(0x104), 0);
        // Back-to-back with no wait states keeps every cycle.
        for i in 0..10u64 {
            m.write(0x1000 + i * 4, i).await?;
        }
        for i in 0..10u64 {
            assert_eq!(m.read(0x1000 + i * 4).await?, i);
        }
        // Timer to make sure nothing is left hanging.
        rivet_core::triggers::Timer::new(50.ns()).await;
        Ok(())
    })
    .unwrap();
}
