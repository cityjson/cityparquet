<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Kanagawa, Yokohama-shi;
  14100_yokohama-shi_city_2024_citygml_2_op), CityGML 2.0 module `tran`,
  tile 53391530_tran_6697_op.gml. Licence: CC BY 4.0 (Project PLATEAU, MLIT Japan).

  Root CityModel + gml:Envelope + ONE cityObjectMember, verbatim; the
  document-level app:appearanceMember is dropped.

  A `tran:Road` with a standalone LoD1 surface (`tran:lod1MultiSurface`, one
  polygon) and, at LoD3, three `tran:TrafficArea` (19 polygons) and one
  `tran:AuxiliaryTrafficArea` (4 polygons), each reached through the
  `tran:trafficArea` / `tran:auxiliaryTrafficArea` properties. CityJSON models
  those as semantic surfaces of the Road's own geometry — the transportation
  counterpart of a Building's `boundedBy`.
-->
<core:CityModel xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:pbase="http://www.opengis.net/citygml/profiles/base/2.0" xmlns:smil20lang="http://www.w3.org/2001/SMIL20/Language" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:smil20="http://www.w3.org/2001/SMIL20/" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:uro="https://www.geospatial.jp/iur/uro/3.1" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:sch="http://www.ascc.net/xml/schematron" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xsi:schemaLocation="https://www.geospatial.jp/iur/uro/3.1 ../../schemas/iur/uro/3.1/urbanObject.xsd http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/gml http://schemas.opengis.net/gml/3.1.1/base/gml.xsd http://www.opengis.net/citygml/cityfurniture/2.0 http://schemas.opengis.net/citygml/cityfurniture/2.0/cityFurniture.xsd http://www.opengis.net/citygml/vegetation/2.0 http://schemas.opengis.net/citygml/vegetation/2.0/vegetation.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd">
	<gml:boundedBy>
		<gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3">
			<gml:lowerCorner>35.438523832599955 139.62165268559278 0</gml:lowerCorner>
			<gml:upperCorner>35.45037647271439 139.63789961437809 5.3916268833937</gml:upperCorner>
		</gml:Envelope>
	</gml:boundedBy>
	<core:cityObjectMember>
		<tran:Road gml:id="tran_91015f0f-07ae-4db4-8f7b-bba28b4496ef">
			<core:creationDate>2025-03-21</core:creationDate>
			<tran:function codeSpace="../../codelists/TrafficArea_function.xml">1020</tran:function>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_baa6750b-07bd-4a15-8570-312bfd545eac">
					<core:creationDate>2025-03-21</core:creationDate>
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">1020</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_67b5b3d3-6e1c-4c84-a327-4efc87887283">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463750700523 139.63610273956363 1.388498045269408 35.44461174845967 139.63609925973017 1.4731645997750342 35.44463707876113 139.63612224416886 1.406831379900384 35.44463750700523 139.63610273956363 1.388498045269408</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_275c904b-24c2-453f-9c03-fc55675a2310">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44461174845967 139.63609925973017 1.4731645997750342 35.44463750700523 139.63610273956363 1.388498045269408 35.4446194977692 139.63608639818443 1.4403835945066068 35.44461174845967 139.63609925973017 1.4731645997750342</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_31c72c1d-8941-4adf-ba8b-90e68e162834">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463750700523 139.63610273956363 1.388498045269408 35.44463707876113 139.63612224416886 1.406831379900384 35.44464482807308 139.6361093826213 1.3806075547607761 35.44463750700523 139.63610273956363 1.388498045269408</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_87fdd283-a7a8-4727-ab59-4978926adc52">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463707876113 139.63612224416886 1.406831379900384 35.44461174845967 139.63609925973017 1.4731645997750342 35.44459518738857 139.63612674615956 1.4254978706576011 35.44463707876113 139.63612224416886 1.406831379900384</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_196d4cc0-9bf0-4431-a8bf-3d89a7fea427">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463707876113 139.63612224416886 1.406831379900384 35.44459518738857 139.63612674615956 1.4254978706576011 35.444620543596976 139.63614968759592 1.3584980431460862 35.44463707876113 139.63612224416886 1.406831379900384</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_5bb7cb0a-24e8-47f3-b3aa-3d346c703355">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444620543596976 139.63614968759592 1.3584980431460862 35.44459518738857 139.63612674615956 1.4254978706576011 35.44457101202163 139.63616687000578 1.35949813178555 35.444620543596976 139.63614968759592 1.3584980431460862</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_dbb75038-223f-453f-9c77-7e0f4b4d6212">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444620543596976 139.63614968759592 1.3584980431460862 35.44457101202163 139.63616687000578 1.35949813178555 35.44459547338067 139.6361912966154 1.2694981254116138 35.444620543596976 139.63614968759592 1.3584980431460862</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_40a30662-d458-409f-8a2d-c37c9ce44baa">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44459547338067 139.6361912966154 1.2694981254116138 35.44457101202163 139.63616687000578 1.35949813178555 35.44458360786558 139.6362050456046 1.2828314596923769 35.44459547338067 139.6361912966154 1.2694981254116138</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_6d2e8169-6967-445b-8502-ed74ac13220c">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44456311209611 139.6361874117244 1.3216663641681354 35.44457101202163 139.63616687000578 1.35949813178555 35.44456016366921 139.6361848749966 1.3199996973875823 35.44456311209611 139.6361874117244 1.3216663641681354</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_80b79639-3691-4769-913a-a136b38fd726">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44457101202163 139.63616687000578 1.35949813178555 35.44456311209611 139.6361874117244 1.3216663641681354 35.44458360786558 139.6362050456046 1.2828314596923769 35.44457101202163 139.63616687000578 1.35949813178555</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_ef45e7b1-5bf6-480f-b9af-b07479129e2a">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44459547338067 139.6361912966154 1.2694981254116138 35.44458360786558 139.6362050456046 1.2828314596923769 35.444585966606766 139.63620707498802 1.2711647921999376 35.44459547338067 139.6361912966154 1.2694981254116138</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_7400f0c0-8bfe-4e0f-b588-ad670ac5e286">
					<core:creationDate>2025-03-21</core:creationDate>
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_c78719e1-a15b-43da-8eb6-b1c34c01aa31">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463750700523 139.63610273956363 1.388498045269408 35.44464562975487 139.6361080520682 1.4306122816996212 35.44463831307277 139.63610141299023 1.4478809323915292 35.44463750700523 139.63610273956363 1.388498045269408</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_a3f7b452-5806-4d75-a0e8-956bde30bb58">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44464562975487 139.6361080520682 1.4306122816996212 35.44463750700523 139.63610273956363 1.388498045269408 35.44464482807308 139.6361093826213 1.3806075547607843 35.44464562975487 139.6361080520682 1.4306122816996212</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_2ad4acce-4d64-49da-92a5-9458841f7f96">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463750700523 139.63610273956363 1.388498045269408 35.44463831307277 139.63610141299023 1.4478809323915292 35.4446194977692 139.63608639818443 1.440383594508403 35.44463750700523 139.63610273956363 1.388498045269408</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_3e377ada-bdb3-4e87-86c9-7cf577643fe3">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.4446194977692 139.63608639818443 1.440383594508403 35.44463831307277 139.63610141299023 1.4478809323915292 35.44462029945075 139.63608506763157 1.4903962302788802 35.4446194977692 139.63608639818443 1.440383594508403</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_caf67fb2-aa87-4f3b-a71a-a0877d4fae06">
					<core:creationDate>2025-03-21</core:creationDate>
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_a710ef9e-d309-402b-b2ae-ca83e5be9306">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44465204320872 139.6360974076428 1.5338068210484155 35.444680223258864 139.63604596385616 1.5721848703893944 35.444644838382374 139.63609087006247 1.5468313898096562 35.44465204320872 139.6360974076428 1.5338068210484155</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_19eac165-c417-4c49-a1da-149ad13058aa">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444680223258864 139.63604596385616 1.5721848703893944 35.44465204320872 139.6360974076428 1.5338068210484155 35.44468204913357 139.63604760669088 1.5700311036207717 35.444680223258864 139.63604596385616 1.5721848703893944</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_bd982e44-a58c-4f20-9faf-ef8620479368">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444644838382374 139.63609087006247 1.5468313898096562 35.444680223258864 139.63604596385616 1.5721848703893944 35.444656529377006 139.63602493667935 1.599999999999607 35.444644838382374 139.63609087006247 1.5468313898096562</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_47037f9b-5776-4157-b1e8-37f66bfb4523">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444644838382374 139.63609087006247 1.5468313898096562 35.444656529377006 139.63602493667935 1.599999999999607 35.4446267129026 139.6360744232077 1.5785119411626256 35.444644838382374 139.63609087006247 1.5468313898096562</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:auxiliaryTrafficArea>
				<tran:AuxiliaryTrafficArea gml:id="atr_ebeaa39e-778b-4c24-9e56-9df2027f75df">
					<core:creationDate>2025-03-21</core:creationDate>
					<tran:function codeSpace="../../codelists/AuxiliaryTrafficArea_function.xml">5000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_03c18ca4-e724-43d9-b3fb-d1475512349c">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.444644838382374 139.63609087006247 1.5468313898096562 35.44464562975487 139.6361080520682 1.4306122816996212 35.44465204320872 139.6360974076428 1.5338068210493803 35.444644838382374 139.63609087006247 1.5468313898096562</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_3a1efd08-ca4c-4e3e-bbb7-05b76a21e276">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44464562975487 139.6361080520682 1.4306122816996212 35.444644838382374 139.63609087006247 1.5468313898096562 35.44463831307277 139.63610141299023 1.4478809323915292 35.44464562975487 139.6361080520682 1.4306122816996212</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_d7e8fc77-8648-45a8-b393-df4489b4a67c">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463831307277 139.63610141299023 1.4478809323915292 35.444644838382374 139.63609087006247 1.5468313898096562 35.4446267129026 139.6360744232077 1.5785119411626256 35.44463831307277 139.63610141299023 1.4478809323915292</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_1487ed8f-7dae-4b21-bd2d-3f9c78de82b9">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.44463831307277 139.63610141299023 1.4478809323915292 35.4446267129026 139.6360744232077 1.5785119411626256 35.44462029945075 139.63608506763157 1.4903962302788802 35.44463831307277 139.63610141299023 1.4478809323915292</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:AuxiliaryTrafficArea>
			</tran:auxiliaryTrafficArea>
			<tran:lod1MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon>
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList>35.44461427219001 139.63623449339826 0 35.444706412286784 139.6360696051073 0 35.444653869544204 139.63602227711874 0 35.44456200715807 139.63618165372222 0 35.444572920195625 139.63623945305883 0 35.44461427219001 139.63623449339826 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod1MultiSurface>
			<tran:lod3MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:CompositeSurface>
							<gml:surfaceMember xlink:href="#poly_67b5b3d3-6e1c-4c84-a327-4efc87887283"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_275c904b-24c2-453f-9c03-fc55675a2310"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_31c72c1d-8941-4adf-ba8b-90e68e162834"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_87fdd283-a7a8-4727-ab59-4978926adc52"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_196d4cc0-9bf0-4431-a8bf-3d89a7fea427"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_5bb7cb0a-24e8-47f3-b3aa-3d346c703355"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_dbb75038-223f-453f-9c77-7e0f4b4d6212"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_40a30662-d458-409f-8a2d-c37c9ce44baa"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_6d2e8169-6967-445b-8502-ed74ac13220c"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_80b79639-3691-4769-913a-a136b38fd726"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_ef45e7b1-5bf6-480f-b9af-b07479129e2a"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_c78719e1-a15b-43da-8eb6-b1c34c01aa31"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_a3f7b452-5806-4d75-a0e8-956bde30bb58"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_2ad4acce-4d64-49da-92a5-9458841f7f96"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_3e377ada-bdb3-4e87-86c9-7cf577643fe3"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_a710ef9e-d309-402b-b2ae-ca83e5be9306"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_19eac165-c417-4c49-a1da-149ad13058aa"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_bd982e44-a58c-4f20-9faf-ef8620479368"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_47037f9b-5776-4157-b1e8-37f66bfb4523"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_03c18ca4-e724-43d9-b3fb-d1475512349c"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_3a1efd08-ca4c-4e3e-bbb7-05b76a21e276"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_d7e8fc77-8648-45a8-b393-df4489b4a67c"></gml:surfaceMember>
							<gml:surfaceMember xlink:href="#poly_1487ed8f-7dae-4b21-bd2d-3f9c78de82b9"></gml:surfaceMember>
						</gml:CompositeSurface>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod3MultiSurface>
			<uro:tranDataQualityAttribute>
				<uro:DataQualityAttribute>
					<uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
					<uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod3>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">000</uro:thematicSrcDesc>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
					<uro:appearanceSrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">5</uro:appearanceSrcDescLod3>
					<uro:lodType codeSpace="../../codelists/Road_lodType.xml">3.2</uro:lodType>
					<uro:publicSurveyDataQualityAttribute>
						<uro:PublicSurveyDataQualityAttribute>
							<uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod1>
							<uro:srcScaleLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">3</uro:srcScaleLod3>
							<uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod1>
							<uro:publicSurveySrcDescLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">011</uro:publicSurveySrcDescLod3>
						</uro:PublicSurveyDataQualityAttribute>
					</uro:publicSurveyDataQualityAttribute>
				</uro:DataQualityAttribute>
			</uro:tranDataQualityAttribute>
		</tran:Road>
	</core:cityObjectMember>
</core:CityModel>
